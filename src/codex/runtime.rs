#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexModelEntry {
    pub name: String,
    pub is_default: bool,
    pub aliases: Vec<String>,
    pub description: Option<String>,
    pub description_zh: Option<String>,
    pub description_en: Option<String>,
}
