//! AT-SPI2 → Slug enum mappings (SEMANTIC-SCHEMA §3).
//!
//! `map_role`/`map_state` translate the raw `atspi` enums into canonical
//! `slug-core` enums. `refine_role` applies the §3.1 Slug *extensions* that
//! promote an AT-SPI role+state/attribute combination into a more specific Slug
//! role (e.g. `ENTRY` + password → `ENTRY_PASSWORD`).

use std::collections::HashMap;

use atspi::{Role as AtspiRole, State as AtspiState, StateSet};
use slug_core::{SlugRole, SlugState};

/// Map a raw AT-SPI2 role to its canonical [`SlugRole`] (§3.1, AT-SPI column).
///
/// Roles with no direct Slug equivalent fall back to [`SlugRole::Generic`]
/// (the §3.1 catch-all container); `Invalid`/`Unknown` map to
/// [`SlugRole::Unknown`].
pub fn map_role(role: AtspiRole) -> SlugRole {
    use AtspiRole as A;
    match role {
        A::Alert => SlugRole::Alert,
        A::Application => SlugRole::Application,
        A::Article => SlugRole::Article,
        A::Button => SlugRole::Button, // ATSPI_ROLE_PUSH_BUTTON (43)
        A::ToggleButton => SlugRole::ButtonToggle,
        A::PushButtonMenu => SlugRole::PopupButton,
        A::Canvas => SlugRole::Canvas,
        A::Caption => SlugRole::Caption,
        A::TableCell => SlugRole::Cell,
        A::CheckBox => SlugRole::Checkbox, // (7)
        A::CheckMenuItem => SlugRole::MenuItemCheckbox,
        A::ColorChooser => SlugRole::ColorChooser,
        A::ColumnHeader | A::TableColumnHeader => SlugRole::ColumnHeader,
        A::ComboBox => SlugRole::ComboBox,
        A::DateEditor => SlugRole::DateEditor,
        A::Definition => SlugRole::Definition,
        A::DescriptionList => SlugRole::DescriptionList,
        A::DescriptionTerm => SlugRole::DescriptionTerm,
        A::DescriptionValue => SlugRole::DescriptionValue,
        A::Dialog => SlugRole::Dialog,
        A::DirectoryPane => SlugRole::DirectoryPane,
        A::DocumentFrame
        | A::DocumentText
        | A::DocumentWeb
        | A::DocumentEmail
        | A::DocumentSpreadsheet
        | A::DocumentPresentation => SlugRole::Document,
        A::Embedded => SlugRole::Embedded,
        A::Entry => SlugRole::Entry, // (79)
        A::Editbar | A::Autocomplete => SlugRole::Entry,
        A::PasswordText => SlugRole::EntryPassword,
        A::Text => SlugRole::EntryMultiline, // (61) refined to StaticText if not editable
        A::Filler => SlugRole::Filler,
        A::Footer => SlugRole::Footer,
        A::Footnote => SlugRole::Footnote,
        A::Form => SlugRole::Form,
        A::Section => SlugRole::Section,
        A::Grouping => SlugRole::Group,
        A::Header => SlugRole::Header,
        A::Heading => SlugRole::Heading,
        A::Icon | A::Image | A::ImageMap | A::CHART => SlugRole::Image,
        A::Label => SlugRole::Label,
        A::Landmark => SlugRole::Landmark,
        A::Link => SlugRole::Link, // (88)
        A::List => SlugRole::List,
        A::ListBox => SlugRole::ListBox,
        A::ListItem => SlugRole::ListItem,
        A::Log => SlugRole::Log,
        A::Marquee => SlugRole::Marquee,
        A::Math | A::MathFraction | A::MathRoot => SlugRole::Math,
        A::Audio | A::Video => SlugRole::Media,
        A::Menu => SlugRole::Menu,
        A::MenuBar => SlugRole::MenuBar,
        A::MenuItem => SlugRole::MenuItem, // (35)
        A::RadioMenuItem => SlugRole::MenuItemRadio,
        A::LevelBar => SlugRole::Meter,
        A::Comment => SlugRole::Note,
        A::Notification => SlugRole::Notification,
        A::PageTab => SlugRole::PageTab,
        A::PageTabList => SlugRole::PageTabList,
        A::Paragraph => SlugRole::Paragraph,
        A::Panel | A::OptionPane | A::Viewport => SlugRole::Panel,
        A::ProgressBar => SlugRole::ProgressBar,
        A::RadioButton => SlugRole::RadioButton,
        A::TableRow => SlugRole::Row,
        A::RowHeader | A::TableRowHeader => SlugRole::RowHeader,
        A::ScrollBar => SlugRole::ScrollBar,
        A::ScrollPane => SlugRole::ScrollPane,
        A::Separator => SlugRole::Separator,
        A::Slider => SlugRole::Slider, // (51)
        A::SpinButton => SlugRole::SpinButton,
        A::SplitPane => SlugRole::Splitter,
        A::Static => SlugRole::StaticText,
        A::StatusBar => SlugRole::StatusBar,
        A::Subscript => SlugRole::Subscript,
        A::Superscript => SlugRole::Superscript,
        A::Suggestion => SlugRole::Suggestion,
        A::Table => SlugRole::Table,
        A::TreeTable | A::Tree => SlugRole::Tree,
        A::TreeItem => SlugRole::TreeItem,
        A::Timer => SlugRole::Timer,
        A::TitleBar => SlugRole::TitleBar,
        A::ToolBar => SlugRole::ToolBar,
        A::ToolTip => SlugRole::ToolTip,
        A::ContentDeletion => SlugRole::ContentDeletion,
        A::ContentInsertion => SlugRole::ContentInsertion,
        A::Frame | A::Window | A::InternalFrame => SlugRole::Window, // FRAME → WINDOW
        A::Invalid | A::Unknown | A::Extended => SlugRole::Unknown,
        // Anonymous/structural containers and anything not individually mapped.
        _ => SlugRole::Generic,
    }
}

/// Apply the §3.1 Slug role *extensions* that depend on state/attributes.
///
/// * `ENTRY` + password semantics → `ENTRY_PASSWORD`
/// * `ENTRY` + `xml-roles`/`role-description` = search → `ENTRY_SEARCH`
/// * `TEXT` mapped to `ENTRY_MULTILINE` but not editable → `STATIC_TEXT`
/// * `TOGGLE_BUTTON` with switch semantics → `SWITCH`
/// * `DIALOG`/`ALERT` + `MODAL` → `ALERT_DIALOG`
pub fn refine_role(
    base: SlugRole,
    states: &[SlugState],
    attributes: &HashMap<String, String>,
) -> SlugRole {
    let has = |s: SlugState| states.contains(&s);
    let role_desc = attributes
        .get("xml-roles")
        .or_else(|| attributes.get("role"))
        .or_else(|| attributes.get("role-description"))
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match base {
        // TEXT (61) defaults to multiline entry; demote to static text when the
        // widget is not user-editable.
        SlugRole::EntryMultiline if !has(SlugState::Editable) => SlugRole::StaticText,
        SlugRole::Entry | SlugRole::EntryMultiline => {
            if role_desc.contains("search") {
                SlugRole::EntrySearch
            } else if role_desc.contains("password") {
                SlugRole::EntryPassword
            } else {
                base
            }
        }
        SlugRole::ButtonToggle if role_desc.contains("switch") => SlugRole::Switch,
        SlugRole::Dialog | SlugRole::Alert if has(SlugState::Modal) => SlugRole::AlertDialog,
        _ => base,
    }
}

/// Map a single AT-SPI2 state to a [`SlugState`], or `None` if it has no Slug
/// equivalent (§3.2: deprecated/unused states are dropped).
pub fn map_state(state: AtspiState) -> Option<SlugState> {
    use AtspiState as S;
    Some(match state {
        S::Active => SlugState::Active,
        S::Animated => SlugState::Animated,
        S::Busy => SlugState::Busy,
        S::Checked => SlugState::Checked, // (4)
        S::Collapsed => SlugState::Collapsed,
        S::IsDefault => SlugState::Default,
        S::Defunct => SlugState::Defunct,
        S::Editable => SlugState::Editable, // (7)
        S::Enabled => SlugState::Enabled,   // (8)
        S::Expanded => SlugState::Expanded,
        S::Focusable => SlugState::Focusable,
        S::Focused => SlugState::Focused, // (12)
        S::HasPopup => SlugState::HasPopup,
        S::Horizontal => SlugState::Horizontal,
        S::Iconified => SlugState::Iconified,
        S::Indeterminate => SlugState::Indeterminate,
        S::InvalidEntry => SlugState::InvalidEntry,
        S::ManagesDescendants => SlugState::ManagesDescendants,
        S::Modal => SlugState::Modal,
        S::MultiLine => SlugState::MultiLine,
        S::Multiselectable => SlugState::MultiSelectable,
        S::Opaque => SlugState::Opaque,
        S::Pressed => SlugState::Pressed,
        S::Required => SlugState::Required,
        S::Resizable => SlugState::Resizable,
        S::Selectable => SlugState::Selectable,
        S::Selected => SlugState::Selected,
        S::Sensitive => SlugState::Sensitive, // (24)
        S::Showing => SlugState::Showing,      // (25)
        S::SingleLine => SlugState::SingleLine,
        S::Stale => SlugState::Stale,
        S::SupportsAutocompletion => SlugState::SupportsAutocompletion,
        S::Transient => SlugState::Transient,
        S::Truncated => SlugState::Truncated,
        S::Vertical => SlugState::Vertical,
        S::Visible => SlugState::Visible, // (30)
        S::Visited => SlugState::Visited,
        // No Slug mapping (deprecated/unused or redundant — §3.2).
        S::Invalid
        | S::Armed
        | S::Expandable
        | S::HasTooltip
        | S::SelectableText
        | S::Checkable
        | S::ReadOnly => return None,
        // Forward-compat: any AT-SPI state we don't model maps to nothing.
        _ => return None,
    })
}

/// Map a whole [`StateSet`] to a sorted, de-duplicated `Vec<SlugState>`.
pub fn map_states(states: StateSet) -> Vec<SlugState> {
    let mut out: Vec<SlugState> = states.iter().filter_map(map_state).collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_role_numbers_map_as_documented() {
        assert_eq!(map_role(AtspiRole::Button), SlugRole::Button);
        assert_eq!(map_role(AtspiRole::Entry), SlugRole::Entry);
        assert_eq!(map_role(AtspiRole::CheckBox), SlugRole::Checkbox);
        assert_eq!(map_role(AtspiRole::MenuItem), SlugRole::MenuItem);
        assert_eq!(map_role(AtspiRole::Slider), SlugRole::Slider);
        assert_eq!(map_role(AtspiRole::Link), SlugRole::Link);
        assert_eq!(map_role(AtspiRole::Frame), SlugRole::Window);
    }

    #[test]
    fn unmapped_roles_fall_back_to_generic() {
        assert_eq!(map_role(AtspiRole::Terminal), SlugRole::Generic);
        assert_eq!(map_role(AtspiRole::Unknown), SlugRole::Unknown);
    }

    #[test]
    fn state_numbers_map_as_documented() {
        assert_eq!(map_state(AtspiState::Focused), Some(SlugState::Focused));
        assert_eq!(map_state(AtspiState::Enabled), Some(SlugState::Enabled));
        assert_eq!(map_state(AtspiState::Checked), Some(SlugState::Checked));
        assert_eq!(map_state(AtspiState::Editable), Some(SlugState::Editable));
        assert_eq!(map_state(AtspiState::Sensitive), Some(SlugState::Sensitive));
        assert_eq!(map_state(AtspiState::Showing), Some(SlugState::Showing));
        assert_eq!(map_state(AtspiState::Visible), Some(SlugState::Visible));
        assert_eq!(map_state(AtspiState::Armed), None);
    }

    #[test]
    fn text_without_editable_is_static() {
        let r = refine_role(SlugRole::EntryMultiline, &[], &HashMap::new());
        assert_eq!(r, SlugRole::StaticText);
        let r2 = refine_role(SlugRole::EntryMultiline, &[SlugState::Editable], &HashMap::new());
        assert_eq!(r2, SlugRole::EntryMultiline);
    }

    #[test]
    fn switch_promotion() {
        let mut attrs = HashMap::new();
        attrs.insert("xml-roles".to_string(), "switch".to_string());
        assert_eq!(refine_role(SlugRole::ButtonToggle, &[], &attrs), SlugRole::Switch);
    }
}
