//! Agent-friendly rendering of the `buzz` command tree.
//!
//! `buzz --help` lists every group *and* the subcommands under it, so a caller
//! learns the whole surface in one invocation instead of one `--help` call per
//! group. The tree is walked from clap's own command definition, so it cannot
//! drift from the commands that actually exist.

use clap::Command;

/// Column (0-indexed, in characters) where descriptions begin.
const DESC_COL: usize = 32;

/// Total line width. This matches clap's own default when its `wrap_help`
/// feature is off, which is how this crate builds clap.
const LINE_WIDTH: usize = 100;

/// Render the subcommand tree under `cmd`, descending at most `max_depth`
/// levels. `max_depth == 1` lists only the top-level groups.
///
/// The returned string has one entry per line and no trailing newline.
pub(crate) fn render(cmd: &Command, max_depth: usize) -> String {
    let mut out = String::new();
    write_level(&mut out, cmd, 1, max_depth);
    out.truncate(out.trim_end().len());
    out
}

fn write_level(out: &mut String, parent: &Command, depth: usize, max_depth: usize) {
    if depth > max_depth {
        return;
    }
    let mut children: Vec<&Command> = parent
        .get_subcommands()
        // clap's auto-generated `help` subcommand tells a reader nothing about
        // what the CLI can do, and it repeats at every level of the tree.
        .filter(|child| !child.is_hide_set() && child.get_name() != "help")
        .collect();
    // Match clap's own ordering: the derive assigns a display order per
    // variant, so this preserves declaration order, with the name as tiebreak.
    children.sort_by_key(|child| (child.get_display_order(), child.get_name()));

    for child in children {
        write_entry(out, child, depth);
        write_level(out, child, depth + 1, max_depth);
    }
}

fn write_entry(out: &mut String, cmd: &Command, depth: usize) {
    let left = format!("{}{}", "  ".repeat(depth), cmd.get_name());
    let description = description_of(cmd);

    if description.is_empty() {
        out.push_str(&left);
        out.push('\n');
        return;
    }

    let pad = DESC_COL.saturating_sub(left.chars().count()).max(1);
    let mut lines = wrap(&description, LINE_WIDTH.saturating_sub(DESC_COL)).into_iter();

    // `wrap` returns at least one line for non-empty input, but fall back to
    // the bare name rather than dropping the entry if that ever changes.
    let Some(first) = lines.next() else {
        out.push_str(&left);
        out.push('\n');
        return;
    };
    out.push_str(&left);
    out.push_str(&" ".repeat(pad));
    out.push_str(&first);
    out.push('\n');

    for line in lines {
        out.push_str(&" ".repeat(DESC_COL));
        out.push_str(&line);
        out.push('\n');
    }
}

/// A command's one-line description, prefixed with its visible aliases.
fn description_of(cmd: &Command) -> String {
    let mut description = String::new();
    let aliases: Vec<&str> = cmd.get_visible_aliases().collect();
    if !aliases.is_empty() {
        description.push_str(&format!("[alias: {}] ", aliases.join(", ")));
    }
    if let Some(about) = cmd.get_about() {
        description.push_str(&about.to_string());
    }
    description.trim_end().to_string()
}

/// Greedy word wrap. Words longer than `width` overflow rather than break.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_command;

    // These assertions are deliberately column-exact. clap runs
    // `StyledStr::wrap` over `after_help`, which is a no-op only because this
    // crate builds clap without the `wrap_help` feature; if workspace feature
    // unification ever turns that on, clap reflows the tree and destroys the
    // alignment. A looser `contains("send")` assertion would not notice.
    #[test]
    fn long_help_lists_each_group_with_its_subcommands() {
        let help = build_command().render_long_help().to_string();

        assert!(
            help.contains(
                "\n  messages                      Send, read, search, and manage messages\n"
            ),
            "group line missing or misaligned:\n{help}"
        );
        assert!(
            help.contains("\n    send                        Send a message to a channel\n"),
            "subcommand line missing or misaligned:\n{help}"
        );
        assert!(
            help.contains("\n    draft-create                Open a prefilled create-agent form in the owner's Buzz Desktop\n"),
            "subcommand line missing or misaligned:\n{help}"
        );
    }

    #[test]
    fn long_help_descends_past_the_second_level() {
        let help = build_command().render_long_help().to_string();

        assert!(
            help.contains("\n    protect                     Manage branch and tag protection rules on one of your repositories\n"),
            "nested group line missing or misaligned:\n{help}"
        );
        assert!(
            help.contains(
                "\n      list                      List the repository's protection rules\n"
            ),
            "third-level line missing or misaligned:\n{help}"
        );
    }

    #[test]
    fn long_help_wraps_long_descriptions_into_the_description_column() {
        let help = build_command().render_long_help().to_string();

        assert!(
            help.contains(
                "\n  emoji                         Manage your custom emoji set (workspace palette is the union of all\n                                members' sets)\n"
            ),
            "wrapped description missing or misaligned:\n{help}"
        );
    }

    #[test]
    fn short_help_stays_at_the_group_level() {
        let help = build_command().render_help().to_string();

        assert!(
            help.contains(
                "\n  messages                      Send, read, search, and manage messages\n"
            ),
            "group line missing from short help:\n{help}"
        );
        assert!(
            !help.contains("\n    send "),
            "short help should not descend into subcommands:\n{help}"
        );
    }

    #[test]
    fn tree_omits_claps_generated_help_subcommand() {
        let tree = render(&build_command(), usize::MAX);

        assert!(!tree.lines().any(|line| line.trim() == "help"));
        assert!(!tree.contains("Print this message or the help of the given subcommand"));
    }

    #[test]
    fn depth_one_lists_groups_only() {
        let cmd = build_command();
        let groups = render(&cmd, 1);

        assert!(groups.contains("\n  messages "));
        assert!(!groups.contains("\n    send "));
    }

    #[test]
    fn wrap_keeps_words_intact_and_respects_width() {
        assert_eq!(wrap("", 10), Vec::<String>::new());
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(
            wrap("supercalifragilistic x", 5),
            vec!["supercalifragilistic", "x"]
        );
    }
}
