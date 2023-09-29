//! Functions to parse the changes from a changelog entry.

use lazy_regex::{regex_captures, regex_replace};
use std::borrow::Cow;

// A specific section in a changelog entry, e.g.:
//
// ```
// [ Joe Example]
// * Foo, bar
//  + Blah
// * Foo
// * Foo
// ```
#[derive(Default, Debug, PartialEq, Eq)]
struct Section<'a> {
    // Title of the section, if any
    title: Option<&'a str>,

    // Line numbers of the section
    linenos: Vec<usize>,

    // List of changes in the section
    changes: Vec<Vec<(usize, &'a str)>>,
}

/// Return the different sections from a set of changelog entries.
///
/// # Arguments
/// * `changes`: list of changes from a changelog entry
///
/// # Returns
///
/// An iterator over tuples with:
///    (author, list of line numbers, list of list of (lineno, line) tuples
fn changes_sections<'a>(
    changes: impl Iterator<Item = &'a str>,
) -> impl Iterator<Item = Section<'a>> {
    let mut ret: Vec<Section<'a>> = vec![];
    let mut section = Section::<'a>::default();
    let mut change = Vec::<(usize, &'a str)>::new();

    for (i, line) in changes.enumerate() {
        if line.is_empty() && i == 0 {
            // Skip the first line
            continue;
        }

        if line.is_empty() {
            section.linenos.push(i);
            continue;
        }

        if let Some((_, author)) = regex_captures!(r"^\[ (.*) \]$", line) {
            if !change.is_empty() {
                section.changes.push(change);
                change = Vec::new();
            }
            if !section.linenos.is_empty() {
                ret.push(section);
            }
            section = Section {
                title: Some(author),
                linenos: vec![i],
                changes: vec![],
            };
        } else if !line.starts_with("* ") {
            change.push((i, line));
            section.linenos.push(i);
        } else {
            if !change.is_empty() {
                section.changes.push(change);
            }
            change = vec![(i, line)];
            section.linenos.push(i);
        }
    }
    if !change.is_empty() {
        section.changes.push(change);
    }
    if !section.linenos.is_empty() {
        ret.push(section);
    }

    ret.into_iter()
}

/// Iterate over changes by author
///
/// # Arguments
/// * `changes`: list of changes from a changelog entry
///
/// # Returns
/// An iterator over tuples with:
///   (author, list of line numbers, list of lines)
pub fn changes_by_author<'a>(
    changes: impl Iterator<Item = &'a str>,
) -> impl Iterator<Item = (Option<&'a str>, Vec<usize>, Vec<&'a str>)> {
    changes_sections(changes).flat_map(|section| {
        section
            .changes
            .into_iter()
            .map(|change_entry| {
                let (linenos, lines): (Vec<_>, Vec<_>) = change_entry.into_iter().unzip();
                (section.title, linenos, lines)
            })
            .collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod changes_sections_tests {
    #[test]
    fn test_simple() {
        let iter =
            super::changes_sections(vec!["", "* Change 1", "* Change 2", "  rest", ""].into_iter());
        assert_eq!(
            vec![super::Section {
                title: None,
                linenos: vec![1, 2, 3, 4],
                changes: vec![
                    (vec![(1, "* Change 1")]),
                    (vec![(2, "* Change 2"), (3, "  rest")])
                ]
            }],
            iter.collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_with_header() {
        assert_eq!(
            vec![
                super::Section {
                    title: Some("Author 1"),
                    linenos: vec![1, 2, 3],
                    changes: vec![(vec![(2, "* Change 1")])]
                },
                super::Section {
                    title: Some("Author 2"),
                    linenos: vec![4, 5, 6, 7],
                    changes: vec![(vec![(5, "* Change 2"), (6, "  rest")])]
                },
            ],
            super::changes_sections(
                vec![
                    "",
                    "[ Author 1 ]",
                    "* Change 1",
                    "",
                    "[ Author 2 ]",
                    "* Change 2",
                    "  rest",
                    "",
                ]
                .into_iter()
            )
            .collect::<Vec<_>>()
        );
    }
}

/// Strip a changelog message like debcommit does.
///
/// Takes a list of changes from a changelog entry and applies a transformation
/// so the message is well formatted for a commit message.
///
/// # Arguments:
/// * `changes` - a list of lines from the changelog entry
///
/// # Returns
/// Another list of lines with blank lines stripped from the start and the
/// spaces the start of the lines split if there is only one logical entry.
pub fn strip_for_commit_message(mut changes: Vec<&str>) -> Vec<&str> {
    if changes.is_empty() {
        return vec![];
    }
    while let Some(last) = changes.last() {
        if last.trim().is_empty() {
            changes.pop();
        } else {
            break;
        }
    }

    while let Some(first) = changes.first() {
        if first.trim().is_empty() {
            changes.remove(0);
        } else {
            break;
        }
    }

    let changes = changes
        .into_iter()
        .map(|mut line| loop {
            if line.starts_with("  ") {
                line = &line[2..];
            } else if line.starts_with('\t') {
                line = &line[1..];
            } else {
                break line;
            }
        })
        .collect::<Vec<_>>();

    // Drop bullet points
    let bullet_points_dropped = changes
        .iter()
        .map(|line| {
            let line = line.trim_start();
            if line.starts_with("* ") || line.starts_with("+ ") || line.starts_with("- ") {
                line[1..].trim_start()
            } else {
                line
            }
        })
        .collect::<Vec<_>>();
    if bullet_points_dropped.len() == 1 {
        bullet_points_dropped
    } else {
        changes
    }
}

#[cfg(test)]
mod strip_for_commit_message_tests {
    #[test]
    fn test_no_changes() {
        assert_eq!(super::strip_for_commit_message(vec![]), Vec::<&str>::new());
    }

    #[test]
    fn test_empty_changes() {
        assert_eq!(
            super::strip_for_commit_message(vec![""]),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn test_removes_leading_whitespace() {
        assert_eq!(
            super::strip_for_commit_message(vec!["foo", "bar", "\tbaz", " bang"]),
            vec!["foo", "bar", "baz", " bang"]
        );
    }

    #[test]
    fn test_removes_star_if_one() {
        assert_eq!(super::strip_for_commit_message(vec!["* foo"]), vec!["foo"]);
        assert_eq!(
            super::strip_for_commit_message(vec!["\t* foo"]),
            vec!["foo"]
        );
        assert_eq!(super::strip_for_commit_message(vec!["+ foo"]), vec!["foo"]);
        assert_eq!(super::strip_for_commit_message(vec!["- foo"]), vec!["foo"]);
        assert_eq!(super::strip_for_commit_message(vec!["*  foo"]), vec!["foo"]);
        assert_eq!(
            super::strip_for_commit_message(vec!["*  foo", "   bar"]),
            vec!["*  foo", " bar"]
        );
    }

    #[test]
    fn test_leaves_start_if_multiple() {
        assert_eq!(
            super::strip_for_commit_message(vec!["* foo", "* bar"]),
            vec!["* foo", "* bar"]
        );
        assert_eq!(
            super::strip_for_commit_message(vec!["* foo", "+ bar"]),
            vec!["* foo", "+ bar"]
        );
        assert_eq!(
            super::strip_for_commit_message(vec!["* foo", "bar", "* baz"]),
            vec!["* foo", "bar", "* baz"]
        );
    }
}
