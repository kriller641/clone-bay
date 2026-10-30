//! Probe formations as a few lines of text that players can pass to each
//! other through the clipboard, apart from any settings file.
//!
//! The text is plain so it can be read and checked by eye:
//!
//! ```text
//! formation: Spread (250 km)
//! probe: 0 250 0 4
//! probe: 0 -250 0 4
//! ```
//!
//! Each probe line is its position in kilometres north, west and up of the
//! centre (negative means south, east and down) and then its scan range in
//! AU: the same columns, in the same order, as the editor's table.
//!
//! Reading is strict. Text with anything unexpected in it is refused as a
//! whole, with the line that was wrong, and what it holds must pass the
//! same checks as formations made in the editor.
//!
//! The first version of this wrote files with a header line and comments
//! above the formations. It was replaced, a day later, by just the
//! formations on the clipboard. That header is still accepted when
//! read, so the contents of such a file can be pasted.

use std::fmt::Write as _;

use thiserror::Error;

use crate::formations::{Formation, FormationError, Formations, Probe, AU, KM};

/// The first line of the files the first version wrote. Allowed, not needed.
const OLD_HEADER: &str = "eve-settings probe formations 1";
/// Real text is a few hundred bytes. Anything bigger than this isn't it.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// The most formations one piece of text may hold.
pub const MAX_SHARED: usize = 200;

/// A formation read from text: a name and probes, with no ID yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Shared {
    pub name: String,
    pub probes: Vec<Probe>,
}

/// Why text couldn't be read as formations. Nothing is changed when one of
/// these is returned.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShareError {
    #[error("the text is too long to be probe formations")]
    TooLarge,
    #[error("it doesn't hold probe formations copied from Clone Bay")]
    NotFormations,
    #[error("line {line} isn't understood: {problem}")]
    Line { line: usize, problem: String },
    #[error("it holds more than {MAX_SHARED} formations")]
    TooMany,
    #[error("{0}")]
    Invalid(String),
}

/// `formations` as text: for each, a `formation:` line with its name and a
/// `probe:` line for each probe, with a blank line between formations.
/// Nothing else: no header and no comments.
pub fn to_text(formations: &[&Formation]) -> String {
    let mut text = String::new();
    for formation in formations {
        if !text.is_empty() {
            text.push('\n');
        }
        // Writing to a String can't fail.
        let _ = writeln!(text, "formation: {}", formation.name.trim());
        for probe in &formation.probes {
            let _ = writeln!(
                text,
                "probe: {} {} {} {}",
                tidy(probe.z / KM),
                tidy(probe.x / KM),
                tidy(probe.y / KM),
                tidy(probe.range / AU)
            );
        }
    }
    text
}

/// A number as written in the text, with "-0" written as "0".
fn tidy(number: f64) -> String {
    if number == 0.0 {
        "0".to_owned()
    } else {
        number.to_string()
    }
}

/// Reads formations from text. Every one must be a formation the editor
/// would accept. Blank lines and lines starting with `#` are ignored.
pub fn from_text(text: &str) -> Result<Vec<Shared>, ShareError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ShareError::TooLarge);
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines = text
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));

    let mut shared: Vec<Shared> = Vec::new();
    let mut first = true;
    for (line, content) in lines {
        let is_first = std::mem::replace(&mut first, false);
        let problem = |problem: &str| ShareError::Line {
            line,
            problem: problem.to_owned(),
        };
        if let Some(name) = content.strip_prefix("formation:") {
            if shared.len() >= MAX_SHARED {
                return Err(ShareError::TooMany);
            }
            shared.push(Shared {
                name: name.trim().to_owned(),
                probes: Vec::new(),
            });
        } else if is_first {
            // Whatever is on the clipboard, if it doesn't start the way
            // formations do it is something else altogether.
            if content != OLD_HEADER {
                return Err(ShareError::NotFormations);
            }
        } else if let Some(numbers) = content.strip_prefix("probe:") {
            let Some(formation) = shared.last_mut() else {
                return Err(problem("a probe comes before any formation"));
            };
            let numbers: Option<Vec<f64>> = numbers.split_whitespace().map(plain_number).collect();
            match numbers.as_deref() {
                Some([north, west, up, range]) => formation.probes.push(Probe {
                    x: west * KM,
                    y: up * KM,
                    z: north * KM,
                    range: range * AU,
                }),
                _ => {
                    return Err(problem(
                        "a probe needs four numbers: north, west, up, range",
                    ))
                }
            }
        } else {
            return Err(problem("it should start with \"formation:\" or \"probe:\""));
        }
    }
    if shared.is_empty() {
        return Err(ShareError::NotFormations);
    }
    check(&shared)?;
    Ok(shared)
}

/// A number written with digits, at most one leading minus and at most one
/// decimal point. No exponents, "inf" or "nan".
fn plain_number(text: &str) -> Option<f64> {
    let sensible = text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '-' | '.'));
    if !sensible {
        return None;
    }
    text.parse().ok().filter(|number: &f64| number.is_finite())
}

/// Holds what was read to the same rules as formations made in the editor,
/// by putting it through the editor's own check.
fn check(shared: &[Shared]) -> Result<(), ShareError> {
    let mut formations = Formations::default();
    for formation in shared {
        formations.add(&formation.name, formation.probes.clone());
    }
    formations.validate().map_err(|error| match error {
        FormationError::Invalid(problem) => ShareError::Invalid(problem),
        other => ShareError::Invalid(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formations::Preset;

    fn formation(name: &str, probes: Vec<Probe>) -> Formation {
        Formation::new(1, name, probes)
    }

    #[test]
    fn formations_are_written_as_just_their_lines() {
        let two = formation(
            "Two probes",
            vec![
                // Stored as (west, up, north).
                Probe::at_km(250.0, -10.5, 1000.0, 4.0),
                Probe::at_km(-250.0, 0.0, -0.0, 0.25),
            ],
        );
        let one = formation("  One  ", vec![Probe::at_km(0.0, 0.0, 0.0, 8.0)]);
        // North, west, up, range. No header, no comments; a blank line
        // between formations and none at the start.
        assert_eq!(
            to_text(&[&two, &one]),
            "formation: Two probes\n\
             probe: 1000 250 -10.5 4\n\
             probe: 0 -250 0 0.25\n\
             \n\
             formation: One\n\
             probe: 0 0 0 8\n"
        );
        assert_eq!(to_text(&[]), "");
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let presets: Vec<Formation> = Preset::ALL
            .iter()
            .map(|preset| formation(preset.label(), preset.probes()))
            .collect();
        let odd = formation(
            "  Odd: # one, with spaces  ",
            vec![
                Probe::at_km(123.456, -0.001, 14_959_787_070.0, 32.0),
                Probe::at_km(-14_959_787_070.0, 7.0, 0.5, 0.5),
            ],
        );
        let mut all: Vec<&Formation> = presets.iter().collect();
        all.push(&odd);
        let read = from_text(&to_text(&all)).unwrap();
        assert_eq!(read.len(), all.len());
        for (read, written) in read.iter().zip(&all) {
            // A name loses the spaces around it and nothing else.
            assert_eq!(read.name, written.name.trim());
            assert_eq!(read.probes.len(), written.probes.len());
            for (a, b) in read.probes.iter().zip(&written.probes) {
                // Kilometres and back can differ in the last digit of a
                // number of metres: far less than a millimetre.
                for (a, b) in [(a.x, b.x), (a.y, b.y), (a.z, b.z)] {
                    assert!((a - b).abs() < 1e-3, "{a} vs {b}");
                }
                assert_eq!(a.range, b.range);
            }
        }
        // Whole kilometres come back exactly.
        assert_eq!(read[1].probes, Preset::ALL[1].probes());
    }

    #[test]
    fn text_may_have_comments_blank_lines_and_windows_line_ends() {
        let text = "\u{feff}# a note first\r\n\r\nformation:   Mine  \r\n\
                    # another\r\n\r\n  probe:  1   2\t3  4 \r\n";
        let mine = Shared {
            name: "Mine".to_owned(),
            probes: vec![Probe::at_km(2.0, 3.0, 1.0, 4.0)],
        };
        assert_eq!(from_text(text).unwrap(), std::slice::from_ref(&mine));
        // The header the first version's files began with is allowed, so
        // the contents of one can be pasted.
        let old = "eve-settings probe formations 1\n# Made by eve-settings.\n\n\
                   formation: Mine\nprobe: 1 2 3 4\n";
        assert_eq!(from_text(old).unwrap(), [mine]);
    }

    fn line_problem(text: &str) -> (usize, String) {
        match from_text(text) {
            Err(ShareError::Line { line, problem }) => (line, problem),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn text_that_is_something_else_is_refused() {
        for other in [
            "",
            "\n\n",
            "# only a comment\n",
            "hello there",
            "https://example.com/some/link",
            "probe: 0 0 0 4\n",
            "eve-settings probe formations 2\nformation: a\nprobe: 0 0 0 4\n",
            "Formation: a\nprobe: 0 0 0 4\n",
            // The old header with nothing after it.
            "eve-settings probe formations 1\n",
            "eve-settings probe formations 1\n# nothing\n",
        ] {
            assert_eq!(
                from_text(other),
                Err(ShareError::NotFormations),
                "{other:?}"
            );
        }
    }

    #[test]
    fn anything_unexpected_after_the_start_refuses_the_whole_text() {
        // The old header is only a header on the first line.
        let (line, _) =
            line_problem("formation: a\nprobe: 0 0 0 4\neve-settings probe formations 1\n");
        assert_eq!(line, 3);
        let (line, problem) = line_problem("eve-settings probe formations 1\nprobe: 0 0 0 4\n");
        assert_eq!(
            (line, problem.as_str()),
            (2, "a probe comes before any formation")
        );
        let (line, problem) = line_problem("\nformation: a\nprobe: 0 0 0 4\nhello\n");
        assert_eq!(line, 4);
        assert!(problem.contains("formation:"), "{problem}");
        for bad in [
            "probe:",
            "probe: 0 0 0",
            "probe: 0 0 0 4 5",
            "probe: 0 0 x 4",
            "probe: 0 0 1e3 4",
            "probe: 0 0 inf 4",
            "probe: 0 0 nan 4",
            "probe: 0 0 +5 4",
            "probe: 0 0 1.2.3 4",
            "probe: 0,0,0,4",
        ] {
            let (line, problem) = line_problem(&format!("formation: a\n{bad}\n"));
            assert_eq!(line, 2, "{bad}");
            assert!(problem.contains("four numbers"), "{bad}: {problem}");
        }
    }

    #[test]
    fn what_the_text_holds_must_pass_the_editors_own_checks() {
        let invalid = |text: &str| match from_text(text) {
            Err(ShareError::Invalid(problem)) => problem,
            other => panic!("{text:?}: {other:?}"),
        };
        // No probes, or too many.
        assert!(invalid("formation: a\n").contains("between 1 and 8 probes"));
        let nine = "probe: 0 0 0 4\n".repeat(9);
        assert!(invalid(&format!("formation: a\n{nine}")).contains("between 1 and 8 probes"));
        // A range the game doesn't offer.
        assert!(invalid("formation: a\nprobe: 0 0 0 3\n").contains("scan range"));
        assert!(invalid("formation: a\nprobe: 0 0 0 -4\n").contains("scan range"));
        // Further out than a probe can be placed (100 AU).
        assert!(invalid("formation: a\nprobe: 14959787071 0 0 4\n").contains("too far"));
        from_text("formation: a\nprobe: 14959787070 0 0 4\n").unwrap();
        // No name, or one that is too long.
        assert!(invalid("formation:\nprobe: 0 0 0 4\n").contains("needs a name"));
        let long = "n".repeat(101);
        assert!(invalid(&format!("formation: {long}\nprobe: 0 0 0 4\n")).contains("too long"));
        // The second formation is checked as well as the first.
        let second = invalid("formation: a\nprobe: 0 0 0 4\nformation: b\nprobe: 0 0 0 9\n");
        assert!(
            second.contains("'b'") && second.contains("scan range"),
            "{second}"
        );
    }

    #[test]
    fn there_are_limits_on_how_much_is_read() {
        let many = |count: usize| {
            let body: String = (0..count)
                .map(|n| format!("formation: f{n}\nprobe: 0 0 0 4\n"))
                .collect();
            from_text(&body)
        };
        assert_eq!(many(MAX_SHARED).unwrap().len(), MAX_SHARED);
        assert_eq!(many(MAX_SHARED + 1), Err(ShareError::TooMany));

        // Exactly at the size limit is still read; one byte more is not.
        let head = "formation: a\nprobe: 0 0 0 4\n";
        let padding = "#".repeat(MAX_TEXT_BYTES - head.len());
        assert_eq!(from_text(&format!("{head}{padding}")).unwrap().len(), 1);
        assert_eq!(
            from_text(&format!("{head}{padding}#")),
            Err(ShareError::TooLarge)
        );
    }

    #[test]
    fn errors_are_worded_for_the_user() {
        let line = ShareError::Line {
            line: 7,
            problem: "a probe needs four numbers".to_owned(),
        };
        assert_eq!(
            line.to_string(),
            "line 7 isn't understood: a probe needs four numbers"
        );
        assert_eq!(
            ShareError::TooMany.to_string(),
            "it holds more than 200 formations"
        );
        assert_eq!(
            ShareError::NotFormations.to_string(),
            "it doesn't hold probe formations copied from Clone Bay"
        );
        assert_eq!(
            ShareError::Invalid("Formation 'a': it needs a name".to_owned()).to_string(),
            "Formation 'a': it needs a name"
        );
    }
}
