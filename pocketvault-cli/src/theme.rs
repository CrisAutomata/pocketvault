use dialoguer::theme::ColorfulTheme;

/// The menu UI's look: arrow-key `Select`/`Confirm`/`Input`/`Password`
/// prompts rendered with `[x]`/`[ ]` prefixes (matching a checkbox rather
/// than dialoguer's default `❯` caret) so the highlighted item reads like a
/// picked option.
pub fn menu_theme() -> ColorfulTheme {
    let mut theme = ColorfulTheme::default();
    theme.active_item_prefix = console::style("[x]".to_string()).green().bold();
    theme.inactive_item_prefix = console::style("[ ]".to_string()).dim();
    theme
}
