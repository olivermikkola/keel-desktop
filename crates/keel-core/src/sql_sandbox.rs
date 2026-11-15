use anyhow::{bail, Result};
use rusqlite::{types::ValueRef, Connection};
use std::path::Path;

const MAX_CSV_FILES: usize = 32;
const MAX_CSV_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESULT_ROWS: usize = 200;

/// Runs a SQL query in an isolated in-memory SQLite database.
/// Any `.csv` files in `workspace_root` (top level and `data/`) are auto-mounted
/// as tables named after their filename (e.g. `SELECT * FROM "sales.csv"` or `sales`).
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
    }

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
        let meta = match std::fs::metadata(&path) {
            Ok(m) if m.len() <= MAX_CSV_BYTES => m,
            _ => continue,
        };
        let _ = meta;
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
            // Also expose a bare stem alias when distinct and valid.
            if stem != table_file_name {
                let _ = load_csv_into_table(conn, &stem, &raw);
            }
        }
    }
    Ok(mounted)
}

fn collect_csvs(dir: &Path, out: &mut Vec<PathBufRef>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
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

fn load_csv_into_table(conn: &Connection, table_name: &str, raw: &str) -> Result<()> {
    let mut lines = raw.lines().filter(|l| !l.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return Ok(());
    };
    let headers = parse_csv_row(header_line);
    if headers.is_empty() {
        return Ok(());
    }

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
            format!("{} TEXT", quote_ident(&name))
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

    for line in lines {
        let mut fields = parse_csv_row(line);
        fields.resize(headers.len(), String::new());
        let params: Vec<&dyn rusqlite::ToSql> =
            fields.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
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
            "item,amount\nCoffee,4.50\nTea,3.00\n\"Croissant, Butter\",5.50\n",
        )
        .unwrap();

        let out = run_sql_in_workspace(
            Some(&dir),
            "SELECT item, CAST(amount AS REAL) AS price FROM \"sales.csv\" ORDER BY price DESC",
        )
        .unwrap();
        assert!(out.contains("Croissant, Butter | 5.5"));
        assert!(out.contains("Coffee | 4.5"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
