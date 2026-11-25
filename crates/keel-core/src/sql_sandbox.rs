use anyhow::{bail, Result};
use rusqlite::{types::Value as SqlValue, types::ValueRef, Connection};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

const MAX_CSV_FILES: usize = 32;
const MAX_CSV_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESULT_ROWS: usize = 200;
const MAX_FILES_TABLE_ROWS: usize = 2_000;
const SQL_QUERY_TIMEOUT_SECS: u64 = 5;

/// Runs a SQL query in an isolated in-memory SQLite database.
/// Any `.csv` files in `workspace_root` (top level and `data/`) are auto-mounted
/// as tables named after their filename (e.g. `SELECT * FROM "sales.csv"` or `sales`),
/// with inferred INTEGER/REAL/TEXT column types so numeric comparisons and ORDER BY work naturally.
/// A read-only `files` table also lists workspace files (`path`, `size_bytes`, `extension`).
pub fn run_sql_in_workspace(workspace_root: Option<&Path>, query: &str) -> Result<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        bail!("SQL query must not be empty");
    }

    let conn = Connection::open_in_memory()?;
    conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_ATTACHED, 0);

    let mut mounted = Vec::new();
    if let Some(root) = workspace_root {
        mounted = mount_workspace_csvs(&conn, root)?;
        let _ = mount_workspace_files_table(&conn, root);
    }

    // Bound runaway SQLite queries (e.g. huge cross-joins) with an instant-cancelling watchdog.
    let interrupt = conn.get_interrupt_handle();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) =
            done_rx.recv_timeout(Duration::from_secs(SQL_QUERY_TIMEOUT_SECS))
        {
            interrupt.interrupt();
        }
    });

    let outcome = execute_sql_inner(&conn, trimmed, &mounted);
    drop(done_tx);
    let _ = watchdog.join();
    outcome
}

fn execute_sql_inner(conn: &Connection, trimmed: &str, mounted: &[String]) -> Result<String> {
    let mut stmt = conn.prepare(trimmed)?;
    let col_count = stmt.column_count();

    if col_count == 0 {
        let changed = stmt.execute([])?;
        return Ok(format!(
            "Query executed ({changed} row(s) affected). Mounted CSV tables: {}",
            if mounted.is_empty() {
                "none".to_string()
            } else {
                mounted.join(", ")
            }
        ));
    }

    let headers: Vec<String> = (0..col_count)
        .map(|i| stmt.column_name(i).unwrap_or("col").to_string())
        .collect();

    let mut rows_out = Vec::new();
    let mut rows = stmt.query([])?;
    let mut truncated = false;
    while let Some(row) = rows.next()? {
        if rows_out.len() >= MAX_RESULT_ROWS {
            truncated = true;
            break;
        }
        let mut vals = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let v = match row.get_ref(i)? {
                ValueRef::Null => "NULL".to_string(),
                ValueRef::Integer(n) => n.to_string(),
                ValueRef::Real(f) => format!("{f}"),
                ValueRef::Text(s) => String::from_utf8_lossy(s).to_string(),
                ValueRef::Blob(b) => format!("<blob {} bytes>", b.len()),
            };
            vals.push(v);
        }
        rows_out.push(vals);
    }

    let mut out = String::new();
    out.push_str(&headers.join(" | "));
    out.push('\n');
    out.push_str(&vec!["---"; headers.len()].join(" | "));
    out.push('\n');
    for r in &rows_out {
        out.push_str(&r.join(" | "));
        out.push('\n');
    }
    if truncated {
        out.push_str(&format!("... (truncated at {MAX_RESULT_ROWS} rows)\n"));
    }
    if rows_out.is_empty() {
        out.push_str("(0 rows)\n");
    }
    Ok(out)
}

fn mount_workspace_csvs(conn: &Connection, root: &Path) -> Result<Vec<String>> {
    let mut mounted = Vec::new();
    let mut candidates = Vec::new();
    collect_csvs(root, &mut candidates)?;
    candidates.truncate(MAX_CSV_FILES);

    for path in candidates {
        match std::fs::metadata(&path) {
            Ok(m) if m.len() <= MAX_CSV_BYTES => {}
            _ => continue,
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let table_file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("table.csv")
            .to_string();
        let stem = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("table")
            .to_string();

        if load_csv_into_table(conn, &table_file_name, &raw).is_ok() {
            mounted.push(table_file_name.clone());
            // Expose a bare stem alias as a VIEW instead of duplicating all rows.
            if stem != table_file_name {
                let view_sql = format!(
                    "CREATE VIEW IF NOT EXISTS {} AS SELECT * FROM {}",
                    quote_ident(&stem),
                    quote_ident(&table_file_name)
                );
                let _ = conn.execute(&view_sql, []);
            }
        }
    }
    Ok(mounted)
}

fn mount_workspace_files_table(conn: &Connection, root: &Path) -> Result<()> {
    // Only create `files` if no CSV table/view already claimed that name.
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE lower(name) = 'files'",
        [],
        |r| r.get(0),
    )?;
    if exists > 0 {
        return Ok(());
    }
    conn.execute(
        "CREATE TABLE \"files\" (\"path\" TEXT, \"size_bytes\" INTEGER, \"extension\" TEXT)",
        [],
    )?;
    let mut stmt = conn.prepare(
        "INSERT INTO \"files\" (\"path\", \"size_bytes\", \"extension\") VALUES (?1, ?2, ?3)",
    )?;
    let mut rows = Vec::new();
    collect_workspace_files_meta(root, root, 0, &mut rows);
    for (rel, size, ext) in rows {
        let _ = stmt.execute(rusqlite::params![rel, size as i64, ext]);
    }
    Ok(())
}

fn collect_workspace_files_meta(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(String, u64, String)>,
) {
    if depth > 4 || out.len() >= MAX_FILES_TABLE_ROWS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES_TABLE_ROWS {
            break;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.eq_ignore_ascii_case(".git")
            || name_str == "node_modules"
            || name_str == "target"
        {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            collect_workspace_files_meta(root, &path, depth + 1, out);
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            out.push((rel, size, ext));
        }
    }
}

fn collect_csvs(dir: &Path, out: &mut Vec<PathBufRef>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
            continue;
        }
        let p = entry.path();
        if p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            out.push(p);
        } else if p.is_dir()
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == "data")
        {
            if let Ok(sub) = std::fs::read_dir(&p) {
                for se in sub.flatten() {
                    if se.file_type().is_ok_and(|kind| kind.is_symlink()) {
                        continue;
                    }
                    let sp = se.path();
                    if sp.is_file()
                        && sp
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
                    {
                        out.push(sp);
                    }
                }
            }
        }
    }
    Ok(())
}

type PathBufRef = std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColType {
    Integer,
    Real,
    Text,
}

fn infer_column_types(col_count: usize, records: &[Vec<String>]) -> Vec<ColType> {
    let mut types = vec![ColType::Integer; col_count];
    let mut seen_non_empty = vec![false; col_count];

    for row in records {
        for (i, cell) in row.iter().enumerate().take(col_count) {
            let trimmed = cell.trim();
            if trimmed.is_empty() {
                continue;
            }
            seen_non_empty[i] = true;
            if types[i] == ColType::Text {
                continue;
            }
            // Keep strings with leading zeros (like ZIP codes "00123") or leading '+' (like phone numbers "+358...") as TEXT.
            let leading_zero_code = trimmed.len() > 1
                && trimmed.starts_with('0')
                && !trimmed.starts_with("0.")
                && !trimmed.starts_with("-0.");
            if leading_zero_code || trimmed.starts_with('+') {
                types[i] = ColType::Text;
                continue;
            }
            if types[i] == ColType::Integer && trimmed.parse::<i64>().is_ok() {
                continue;
            }
            if trimmed.parse::<f64>().is_ok_and(|v| v.is_finite()) {
                types[i] = ColType::Real;
            } else {
                types[i] = ColType::Text;
            }
        }
    }

    for i in 0..col_count {
        if !seen_non_empty[i] {
            types[i] = ColType::Text;
        }
    }
    types
}

fn load_csv_into_table(conn: &Connection, table_name: &str, raw: &str) -> Result<()> {
    // Strip a UTF-8 BOM (common in Excel/Numbers exports) so the first column name matches SQL queries.
    let clean = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(clean.as_bytes());
    let headers = reader.headers()?.clone();
    if headers.is_empty() {
        return Ok(());
    }

    let mut records: Vec<Vec<String>> = Vec::new();
    for record in reader.records() {
        let mut fields: Vec<String> = record?.iter().map(String::from).collect();
        fields.resize(headers.len(), String::new());
        records.push(fields);
    }

    let col_types = infer_column_types(headers.len(), &records);
    let safe_table = quote_ident(table_name);
    let col_defs: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let name = if h.trim().is_empty() {
                format!("col_{i}")
            } else {
                h.trim().to_string()
            };
            let sql_type = match col_types[i] {
                ColType::Integer => "INTEGER",
                ColType::Real => "REAL",
                ColType::Text => "TEXT",
            };
            format!("{} {sql_type}", quote_ident(&name))
        })
        .collect();

    conn.execute(
        &format!(
            "CREATE TABLE IF NOT EXISTS {safe_table} ({})",
            col_defs.join(", ")
        ),
        [],
    )?;

    let placeholders = vec!["?"; headers.len()].join(", ");
    let insert_sql = format!("INSERT INTO {safe_table} VALUES ({placeholders})");
    let mut stmt = conn.prepare(&insert_sql)?;

    for fields in &records {
        let values: Vec<SqlValue> = fields
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let trimmed = s.trim();
                match col_types[i] {
                    ColType::Integer => {
                        if trimmed.is_empty() {
                            SqlValue::Null
                        } else if let Ok(n) = trimmed.parse::<i64>() {
                            SqlValue::Integer(n)
                        } else {
                            SqlValue::Text(s.clone())
                        }
                    }
                    ColType::Real => {
                        if trimmed.is_empty() {
                            SqlValue::Null
                        } else if let Ok(f) = trimmed.parse::<f64>() {
                            SqlValue::Real(f)
                        } else {
                            SqlValue::Text(s.clone())
                        }
                    }
                    ColType::Text => SqlValue::Text(s.clone()),
                }
            })
            .collect();
        let params: Vec<&dyn rusqlite::ToSql> =
            values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        stmt.execute(params.as_slice())?;
    }
    Ok(())
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub fn parse_csv_row(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                fields.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur.trim().to_string());
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_csv_and_runs_select() {
        let dir = std::env::temp_dir().join(format!("keel_sql_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sales.csv"),
            "item,amount,zip\nCoffee,4.50,00100\nTea,100.00,00200\n\"Croissant, Butter\",9.50,00300\n",
        )
        .unwrap();

        // Numeric ORDER BY works without explicit CAST (100 > 9.5 > 4.5, not lexicographic "9.5" > "100").
        let out = run_sql_in_workspace(
            Some(&dir),
            "SELECT item, amount, zip FROM sales ORDER BY amount DESC",
        )
        .unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[2].starts_with("Tea | 100 | 00200"), "{out}");
        assert!(
            lines[3].starts_with("Croissant, Butter | 9.5 | 00300"),
            "{out}"
        );
        assert!(lines[4].starts_with("Coffee | 4.5 | 00100"), "{out}");

        // Read-only `files` table lists workspace files.
        let files_out = run_sql_in_workspace(
            Some(&dir),
            "SELECT path, extension FROM files WHERE extension = 'csv'",
        )
        .unwrap();
        assert!(files_out.contains("sales.csv | csv"), "{files_out}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strips_utf8_bom_from_csv_headers() {
        let dir = std::env::temp_dir().join(format!("keel_sql_bom_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("export.csv"), "\u{feff}id,total\n1,42.5\n2,19.0\n").unwrap();

        let out = run_sql_in_workspace(Some(&dir), "SELECT id, total FROM export ORDER BY id ASC")
            .unwrap();
        assert!(out.contains("id | total"), "{out}");
        assert!(out.contains("1 | 42.5"), "{out}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
