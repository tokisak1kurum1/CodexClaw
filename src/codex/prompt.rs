use crate::message::IncomingMessage;
pub(crate) fn build_prompt(message: &IncomingMessage, relevant: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(memory) = relevant.filter(|x| !x.is_empty()) {
        parts.push(format!("<relevant-memory>\n{memory}\n</relevant-memory>"));
    }
    if let Some(quote) = &message.quote {
        parts.push(format!("Quoted message:\n{}", quote.text));
    }
    for file in &message.files {
        parts.push(format!("Attached file: {}", file.local_path.display()));
    }
    parts.push(message.text.clone());
    parts.join("\n\n")
}
