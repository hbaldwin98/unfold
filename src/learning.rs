use crate::provider::{LearningAction, LearningMode, PreviousTurn};

const MAX_PREVIOUS_TURNS: usize = 20;
const MAX_PREVIOUS_TURN_BYTES: usize = 80_000;

#[derive(Clone, Debug)]
pub struct Turn {
    pub label: String,
    pub content: String,
    pub detail: Option<String>,
    pub sources: Vec<Source>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub title: String,
    pub url: String,
}

impl Turn {
    pub fn add_source(&mut self, title: &str, url: &str) {
        if self.sources.iter().any(|source| source.url == url) {
            return;
        }
        let title = strip_controls(title)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        self.sources.push(Source {
            title: if title.is_empty() {
                "Source".into()
            } else {
                title
            },
            url: url.to_owned(),
        });
    }
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
    let visible = turns
        .iter()
        .filter(|turn| !visible_content(&turn.content).trim().is_empty())
        .map(|turn| PreviousTurn {
            label: turn.label.clone(),
            content: visible_content(&turn.content),
            learner_detail: turn.detail.clone(),
        })
        .collect::<Vec<_>>();
    let mut remaining = MAX_PREVIOUS_TURN_BYTES;
    let mut selected = Vec::new();

    for turn in visible.iter().rev().take(MAX_PREVIOUS_TURNS) {
        let length = previous_turn_bytes(turn);
        if length <= remaining {
            selected.push(PreviousTurn {
                label: turn.label.clone(),
                content: turn.content.clone(),
                learner_detail: turn.learner_detail.clone(),
            });
            remaining -= length;
            continue;
        }

        // Only truncate the newest turn. Once an older turn does not fit, stop so
        // the result remains a contiguous suffix of the visible conversation.
        if selected.is_empty() {
            selected.push(truncate_previous_turn(turn, remaining));
        }
        break;
    }
    selected.reverse();
    selected
}

fn previous_turn_bytes(turn: &PreviousTurn) -> usize {
    turn.label.len()
        + turn.content.len()
        + turn
            .learner_detail
            .as_deref()
            .map(str::len)
            .unwrap_or_default()
}

fn truncate_previous_turn(turn: &PreviousTurn, limit: usize) -> PreviousTurn {
    let mut remaining = limit;
    let label = utf8_prefix(&turn.label, remaining).to_owned();
    remaining -= label.len();
    let content = utf8_prefix(&turn.content, remaining).to_owned();
    remaining -= content.len();
    let learner_detail = turn.learner_detail.as_deref().map(|detail| {
        let detail = utf8_prefix(detail, remaining).to_owned();
        remaining -= detail.len();
        detail
    });
    debug_assert_eq!(
        remaining
            + previous_turn_bytes(&PreviousTurn {
                label: label.clone(),
                content: content.clone(),
                learner_detail: learner_detail.clone(),
            }),
        limit
    );
    PreviousTurn {
        label,
        content,
        learner_detail,
    }
}

fn utf8_prefix(value: &str, limit: usize) -> &str {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
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

    #[test]
    fn sources_are_sanitized_and_deduplicated_without_entering_history() {
        let mut turn = Turn {
            label: "Guide".into(),
            content: "Answer".into(),
            detail: None,
            sources: Vec::new(),
        };
        turn.add_source("  Docs\x1b[2J  title ", "https://example.test/a");
        turn.add_source("Duplicate", "https://example.test/a");

        assert_eq!(turn.sources.len(), 1);
        assert_eq!(turn.sources[0].title, "Docs title");
        let history = previous_turns(&[turn]);
        assert_eq!(history[0].content, "Answer");
        assert!(!history[0].content.contains("example.test"));
    }

    fn turn(label: &str, content: String) -> Turn {
        Turn {
            label: label.into(),
            content,
            detail: None,
            sources: Vec::new(),
        }
    }

    #[test]
    fn previous_turns_keep_the_newest_twenty_in_chronological_order() {
        let turns = (0..25)
            .map(|index| turn(&format!("turn-{index}"), "visible".into()))
            .collect::<Vec<_>>();

        let history = previous_turns(&turns);

        assert_eq!(history.len(), 20);
        assert_eq!(history.first().unwrap().label, "turn-5");
        assert_eq!(history.last().unwrap().label, "turn-24");
    }

    #[test]
    fn previous_turns_stop_before_an_older_turn_that_exceeds_the_byte_limit() {
        let turns = vec![
            turn("old", "x".repeat(MAX_PREVIOUS_TURN_BYTES)),
            turn("middle", "middle-content".into()),
            turn("new", "new-content".into()),
        ];

        let history = previous_turns(&turns);

        assert_eq!(history.len(), 2);
        assert_eq!(history[0].label, "middle");
        assert_eq!(history[1].label, "new");
        assert!(history.iter().map(previous_turn_bytes).sum::<usize>() <= 80_000);
    }

    #[test]
    fn oversized_newest_turn_is_utf8_safely_truncated_to_the_provider_limit() {
        let history = previous_turns(&[turn("newest", "界".repeat(30_000))]);

        assert_eq!(history.len(), 1);
        assert_eq!(
            previous_turn_bytes(&history[0]),
            MAX_PREVIOUS_TURN_BYTES - 2
        );
        assert!(
            history[0]
                .content
                .is_char_boundary(history[0].content.len())
        );
        assert!(std::str::from_utf8(history[0].content.as_bytes()).is_ok());
    }
}
