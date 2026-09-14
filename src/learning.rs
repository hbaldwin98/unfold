use crate::provider::{LearningAction, LearningMode, PreviousTurn};

#[derive(Clone, Debug)]
pub struct Turn {
    pub label: String,
    pub content: String,
    pub detail: Option<String>,
}

pub fn label(action: LearningAction, mode: LearningMode) -> &'static str {
    match (action, mode) {
        (LearningAction::Initial, LearningMode::WorkedExample) => "Worked example",
        (LearningAction::Initial, _) => "First question",
        (LearningAction::SocraticResponse, _) => "Next question",
        (LearningAction::FollowUp, _) => "Follow-up",
        (LearningAction::ExplainTerm, _) => "Term explanation",
        (LearningAction::AnotherHint, _) => "Guiding question",
        (LearningAction::ExplainStep, _) => "Step explanation",
        (LearningAction::CheckAttempt, _) => "Attempt feedback",
        (LearningAction::RevealSolution, _) => "Target solution",
    }
}

pub fn previous_turns(turns: &[Turn]) -> Vec<PreviousTurn> {
    turns
        .iter()
        .filter(|turn| !visible_content(&turn.content).trim().is_empty())
        .map(|turn| PreviousTurn {
            label: turn.label.clone(),
            content: visible_content(&turn.content),
            learner_detail: turn.detail.clone(),
        })
        .collect()
}

pub fn visible_content(value: &str) -> String {
    let clean = strip_controls(value);
    let clean = strip_marker(&clean, "SUGGESTIONS");
    let clean = strip_marker(&clean, "TERMS");
    ["think", "analysis", "reasoning"]
        .iter()
        .fold(clean, |text, tag| strip_tag(&text, tag))
        .trim_end()
        .to_owned()
}

fn strip_marker(value: &str, name: &str) -> String {
    let mut text = value.to_owned();
    let needle = format!("<!--{name}");
    while let Some(start) = find_ascii_case(&text, &needle) {
        match text[start..].find("-->") {
            Some(end) => text.replace_range(start..start + end + 3, ""),
            None => {
                text.truncate(start);
                break;
            }
        }
    }
    text
}

fn strip_tag(value: &str, tag: &str) -> String {
    let mut text = value.to_owned();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    while let Some(start) = find_ascii_case(&text, &open) {
        let tail = &text[start..];
        match find_ascii_case(tail, &close) {
            Some(end) => text.replace_range(start..start + end + close.len(), ""),
            None => {
                text.truncate(start);
                break;
            }
        }
    }
    text
}

fn find_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

pub fn strip_controls(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\x1b' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\x07' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\x00'..='\x08' | '\x0b'..='\x0c' | '\x0e'..='\x1f' | '\x7f'..='\u{009f}' => {}
            _ => output.push(character),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_metadata_reasoning_and_partial_markers() {
        assert_eq!(
            visible_content(
                "Visible<think>secret</think> text\n<!--TERMS:[\"x\"]-->\n<!--SUGGESTIONS:"
            ),
            "Visible text"
        );
    }

    #[test]
    fn strips_terminal_control_sequences_but_keeps_text_and_lines() {
        assert_eq!(
            strip_controls("safe\x1b[2J\x1b]0;owned\x07 text\nnext\x00"),
            "safe text\nnext"
        );
    }

    #[test]
    fn labels_every_learning_action() {
        let cases = [
            (
                LearningAction::Initial,
                LearningMode::WorkedExample,
                "Worked example",
            ),
            (
                LearningAction::Initial,
                LearningMode::Socratic,
                "First question",
            ),
            (
                LearningAction::SocraticResponse,
                LearningMode::Socratic,
                "Next question",
            ),
            (
                LearningAction::FollowUp,
                LearningMode::Socratic,
                "Follow-up",
            ),
            (
                LearningAction::ExplainTerm,
                LearningMode::Socratic,
                "Term explanation",
            ),
            (
                LearningAction::AnotherHint,
                LearningMode::Socratic,
                "Guiding question",
            ),
            (
                LearningAction::ExplainStep,
                LearningMode::Socratic,
                "Step explanation",
            ),
            (
                LearningAction::CheckAttempt,
                LearningMode::Socratic,
                "Attempt feedback",
            ),
            (
                LearningAction::RevealSolution,
                LearningMode::Socratic,
                "Target solution",
            ),
        ];
        for (action, mode, expected) in cases {
            assert_eq!(label(action, mode), expected);
        }
    }
}
