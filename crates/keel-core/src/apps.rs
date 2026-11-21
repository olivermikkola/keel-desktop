use serde::{Deserialize, Serialize};

/// Built-in app presets mirroring Keel Mobile, adapted for desktop workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppPreset {
    #[default]
    Chat,
    General,
    Code,
    Websites,
    Sheets,
    Docs,
    Study,
    Slides,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPresetInfo {
    pub id: AppPreset,
    pub title: String,
    pub summary: String,
    pub starters: Vec<String>,
    pub tools: Vec<String>,
    pub mode: String,
}

impl AppPreset {
    pub fn all() -> &'static [AppPreset] {
        &[
            AppPreset::Chat,
            AppPreset::General,
            AppPreset::Code,
            AppPreset::Websites,
            AppPreset::Sheets,
            AppPreset::Docs,
            AppPreset::Study,
            AppPreset::Slides,
        ]
    }

    pub fn title(self) -> &'static str {
        match self {
            AppPreset::Chat => "Chat",
            AppPreset::General => "Work",
            AppPreset::Code => "Code",
            AppPreset::Websites => "Websites",
            AppPreset::Sheets => "Sheets",
            AppPreset::Docs => "Docs",
            AppPreset::Study => "Study",
            AppPreset::Slides => "Slides",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            AppPreset::Chat => "Ask questions, think things through, and write together",
            AppPreset::General => "Turn your files and ideas into finished work",
            AppPreset::Code => {
                "Explore a repository, make changes, and run checks with your approval"
            }
            AppPreset::Websites => "Build a web page and preview it live",
            AppPreset::Sheets => "Query CSV tables with SQL or build spreadsheets",
            AppPreset::Docs => "Write and revise Markdown documents",
            AppPreset::Study => "Generate flashcards and quizzes from notes",
            AppPreset::Slides => "Build an HTML slide deck and export to PDF",
        }
    }

    pub fn starters(self) -> &'static [&'static str] {
        match self {
            AppPreset::Chat => &[
                "Write a friendly text saying I'll be 10 minutes late",
                "Explain how the internet works like I'm 12",
                "Give me 5 easy dinner ideas using rice and eggs",
                "Plan a relaxed weekend in Helsinki",
            ],
            AppPreset::General => &[
                "Summarize the documents in this folder",
                "Turn my notes into a project brief",
                "Analyze the CSV files in this workspace",
            ],
            AppPreset::Code => &[
                "Explore this workspace and summarize how it is structured",
                "Find and fix a bug or failing test in this project",
                "Write a small script or utility in this folder",
            ],
            AppPreset::Websites => &[
                "Make a one-page site for a local cafe, open weekdays 8-16",
                "Make a clean product landing page with dark mode",
                "Make a simple interactive to-do list page",
            ],
            AppPreset::Sheets => &[
                "Make a monthly budget table with rent, food, transport, and savings",
                "Which row in my CSV has the highest total?",
                "Summarize my CSV data by category",
            ],
            AppPreset::Docs => &[
                "Write a one-page project brief in Markdown",
                "Turn my notes into a structured report",
                "Write a clear README for this folder",
            ],
            AppPreset::Study => &[
                "Make flashcards from the notes in this workspace",
                "Quiz me with 5 multiple-choice questions about the solar system",
                "Make 10 flashcards of Finnish phrases for traveling",
            ],
            AppPreset::Slides => &[
                "Make a 5-slide pitch deck for a neighborhood bakery",
                "Turn my notes into a 6-slide presentation",
                "Make a short slide deck explaining how local LLMs work",
            ],
        }
    }

    pub fn allowed_tools(self) -> &'static [&'static str] {
        match self {
            AppPreset::Chat => &[],
            AppPreset::Code => &[
                "list_files",
                "read_file",
                "search_files",
                "propose_edit",
                "create_file",
                "delete_file",
                "rename_file",
                "run_sql",
                "run_shell",
            ],
            AppPreset::Websites => &[
                "list_files",
                "read_file",
                "search_files",
                "propose_edit",
                "create_file",
                "delete_file",
                "rename_file",
            ],
            AppPreset::General | AppPreset::Sheets => &[
                "list_files",
                "read_file",
                "search_files",
                "propose_edit",
                "create_file",
                "delete_file",
                "rename_file",
                "run_sql",
            ],
            AppPreset::Docs | AppPreset::Study | AppPreset::Slides => &[
                "list_files",
                "read_file",
                "search_files",
                "propose_edit",
                "create_file",
                "delete_file",
                "rename_file",
            ],
        }
    }

    /// Read-only CSV questions do not need the full editing/shell schema payload.
    pub fn tools_for_request(self, prompt: &str) -> &'static [&'static str] {
        let prompt = prompt.to_lowercase();
        let writes = [
            "create", "write", "save", "edit", "update", "delete", "rename", "build", "make",
            "plot", "chart", "graph", "export",
        ];
        if matches!(self, Self::General | Self::Sheets)
            && prompt.contains("csv")
            && !writes.iter().any(|word| {
                prompt
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|token| token == *word)
            })
        {
            &["list_files", "read_file", "run_sql"]
        } else {
            self.allowed_tools()
        }
    }

    pub fn instructions(self) -> &'static str {
        match self {
            AppPreset::Chat => {
                "You are Keel, a helpful conversational assistant. Answer questions, explain ideas, brainstorm, and help with writing. You have no file, shell, or connector tools in Chat. Do not claim to access files, browse the web, or execute actions. If the user needs work done in a folder, suggest Work; for repository changes or commands, suggest Code."
            }
            AppPreset::General => {
                "You are Keel in Work, helping turn local files into useful deliverables. Inspect relevant files, make a brief plan for multi-step tasks, then carry out the work. Use SQL for exact calculations over CSV data. File changes are staged for review. Do not claim a deliverable is saved until its edit is approved. Shell commands are available in Code, not Work."
            }
            AppPreset::Code => {
                "You are Keel, a local AI agent working inside the user's selected workspace folder. Inspect files before editing them. Keep changes minimal and accurate. Propose edits with propose_edit or create_file, and only request run_shell when compiling, testing, or running a command is genuinely needed."
            }
            AppPreset::Websites => {
                "This chat is the Websites app. Use list_files to find existing assets/ PNGs; reference them with relative img src paths when appropriate. Build what the user asks as a self-contained web page in index.html (with inline CSS and JavaScript unless separate files are requested). Keep the layout responsive and clean, with real content instead of placeholders, and avoid external network fonts or CDN scripts so it previews reliably offline. To update an existing page, read it first and use propose_edit."
            }
            AppPreset::Sheets => {
                "This chat is the Sheets app. Work with CSV tables in the workspace. To answer questions about a .csv file, query it with run_sql (for example SELECT * FROM \"sales.csv\") and report exact numbers from the result. When asked to create or update a table, write a .csv file using create_file or propose_edit with a header row first, quoting any field containing commas."
            }
            AppPreset::Docs => {
                "This chat is the Docs app. Write documents as clean Markdown (.md) files with a # title, ## sections, concise paragraphs, and lists or tables where helpful. Write complete text rather than placeholders. To revise an existing document, read it first and use propose_edit."
            }
            AppPreset::Study => {
                "This chat is the Study app. Help the user learn a topic or workspace notes by writing a Markdown (.md) flashcard deck. Start with a # title, then for each card write a Q: line and an A: line separated by a blank line. For multiple-choice questions, write the Q: line, choices starting with '- ', and an A: line matching the right choice. Make 8 to 12 cards unless asked otherwise."
            }
            AppPreset::Slides => {
                "This chat is the Slides app. Use existing assets/ PNGs with relative img src paths when useful. Build presentations as a single self-contained slides.html file: put each slide inside a <section class=\"slide\"> element with a clear heading and concise bullets, include compact CSS plus Previous/Next controls inside the file, and add @media print rules so every slide prints cleanly to PDF without external network resources."
            }
        }
    }

    pub fn mode(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Code => "code",
            _ => "work",
        }
    }

    pub fn info(self) -> AppPresetInfo {
        AppPresetInfo {
            id: self,
            mode: self.mode().to_string(),
            title: self.title().to_string(),
            summary: self.summary().to_string(),
            starters: self.starters().iter().map(|s| (*s).to_string()).collect(),
            tools: self
                .allowed_tools()
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_have_starters_and_tools() {
        for preset in AppPreset::all() {
            let info = preset.info();
            assert!(!info.title.is_empty());
            assert!(!info.starters.is_empty());
            if preset != &AppPreset::Chat {
                assert!(info.tools.contains(&"read_file".to_string()));
            }
        }
    }
}
