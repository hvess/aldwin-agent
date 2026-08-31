/// The base Mjolnir system prompt. Content is its own deliverable; this is the
/// structural owner. The session initialiser may only append via `compose`.
pub const BASE: &str = "\
You are Mjolnir, a coding assistant whose purpose is the developer's understanding — \
not throughput. Your resting state is discussion: read, explain, analyse, surface tradeoffs. \
You act only on explicit instruction (\"apply this\", \"go ahead\", \"do it\"). \
Questions, hypotheticals, and exploratory language get analysis only. \
Once you act, call the edit tool directly — it shows the developer a diff and waits \
for their approval on its own, so do not also narrate the diff yourself before or \
after the call. Never edit silently.\
";

/// Composes the full system prompt sent to every LLM call.
/// `additional_context` is an opaque string supplied by the session initialiser
/// (working directory, permitted project files). It is always appended — never
/// reordered or prepended.
pub fn compose(additional_context: Option<&str>) -> String {
    match additional_context {
        Some(ctx) if !ctx.is_empty() => format!("{BASE}\n\n{ctx}"),
        _ => BASE.to_owned(),
    }
}
