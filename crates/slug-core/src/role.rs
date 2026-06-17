//! `SlugRole` — the canonical role enumeration (SEMANTIC-SCHEMA §3.1).
//!
//! Every variant corresponds to exactly one row of the §3.1 table. The JSON wire
//! form is `SCREAMING_SNAKE_CASE` (matching the doc's canonical names, e.g.
//! `BUTTON`, `ENTRY_MULTILINE`). The AT-SPI2 → `SlugRole` mapping itself lives in
//! `slug-bridge`, since it depends on the `atspi` role enum.

use serde::{Deserialize, Serialize};

/// Canonical Slug role. Faithful mirror of the §3.1 enumeration, including the
/// AccessKit-only gaps, AT-SPI2-only gaps, and the five Slug extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SlugRole {
    Alert,
    AlertDialog,
    Application,
    Article,
    Button,
    ButtonToggle,
    Canvas,
    Caption,
    Cell,
    Checkbox,
    ColorChooser,
    ColumnHeader,
    ComboBox,
    Comment,
    ContentDeletion,
    ContentInsertion,
    DateEditor,
    Definition,
    DescriptionList,
    DescriptionTerm,
    DescriptionValue,
    Dialog,
    DirectoryPane,
    Document,
    Embedded,
    Entry,
    EntryMultiline,
    EntryPassword,
    EntrySearch,
    Feed,
    Figure,
    Filler,
    Footer,
    Footnote,
    Form,
    Generic,
    Grid,
    GridCell,
    Group,
    Header,
    Heading,
    Image,
    Inline,
    InputTime,
    Label,
    Landmark,
    Legend,
    LineBreak,
    Link,
    List,
    ListBox,
    ListItem,
    Log,
    Main,
    Marquee,
    Math,
    Media,
    Menu,
    MenuBar,
    MenuItem,
    MenuItemCheckbox,
    MenuItemRadio,
    Meter,
    Navigation,
    Note,
    Notification,
    Option,
    PageTab,
    PageTabList,
    Paragraph,
    Panel,
    PopupButton,
    ProgressBar,
    RadioButton,
    RadioGroup,
    Row,
    RowGroup,
    RowHeader,
    ScrollBar,
    ScrollPane,
    Search,
    Section,
    Separator,
    Slider,
    SpinButton,
    Splitter,
    StaticText,
    StatusBar,
    Strong,
    Subscript,
    Superscript,
    Suggestion,
    Switch,
    Table,
    Term,
    Time,
    Timer,
    TitleBar,
    ToolBar,
    ToolTip,
    Tree,
    TreeItem,
    Window,
    #[serde(other)]
    Unknown,
}

impl Default for SlugRole {
    fn default() -> Self {
        SlugRole::Unknown
    }
}

impl SlugRole {
    /// Canonical `SCREAMING_SNAKE_CASE` name (matches the JSON wire form).
    pub fn as_str(&self) -> &'static str {
        use SlugRole::*;
        match self {
            Alert => "ALERT",
            AlertDialog => "ALERT_DIALOG",
            Application => "APPLICATION",
            Article => "ARTICLE",
            Button => "BUTTON",
            ButtonToggle => "BUTTON_TOGGLE",
            Canvas => "CANVAS",
            Caption => "CAPTION",
            Cell => "CELL",
            Checkbox => "CHECKBOX",
            ColorChooser => "COLOR_CHOOSER",
            ColumnHeader => "COLUMN_HEADER",
            ComboBox => "COMBO_BOX",
            Comment => "COMMENT",
            ContentDeletion => "CONTENT_DELETION",
            ContentInsertion => "CONTENT_INSERTION",
            DateEditor => "DATE_EDITOR",
            Definition => "DEFINITION",
            DescriptionList => "DESCRIPTION_LIST",
            DescriptionTerm => "DESCRIPTION_TERM",
            DescriptionValue => "DESCRIPTION_VALUE",
            Dialog => "DIALOG",
            DirectoryPane => "DIRECTORY_PANE",
            Document => "DOCUMENT",
            Embedded => "EMBEDDED",
            Entry => "ENTRY",
            EntryMultiline => "ENTRY_MULTILINE",
            EntryPassword => "ENTRY_PASSWORD",
            EntrySearch => "ENTRY_SEARCH",
            Feed => "FEED",
            Figure => "FIGURE",
            Filler => "FILLER",
            Footer => "FOOTER",
            Footnote => "FOOTNOTE",
            Form => "FORM",
            Generic => "GENERIC",
            Grid => "GRID",
            GridCell => "GRID_CELL",
            Group => "GROUP",
            Header => "HEADER",
            Heading => "HEADING",
            Image => "IMAGE",
            Inline => "INLINE",
            InputTime => "INPUT_TIME",
            Label => "LABEL",
            Landmark => "LANDMARK",
            Legend => "LEGEND",
            LineBreak => "LINE_BREAK",
            Link => "LINK",
            List => "LIST",
            ListBox => "LIST_BOX",
            ListItem => "LIST_ITEM",
            Log => "LOG",
            Main => "MAIN",
            Marquee => "MARQUEE",
            Math => "MATH",
            Media => "MEDIA",
            Menu => "MENU",
            MenuBar => "MENU_BAR",
            MenuItem => "MENU_ITEM",
            MenuItemCheckbox => "MENU_ITEM_CHECKBOX",
            MenuItemRadio => "MENU_ITEM_RADIO",
            Meter => "METER",
            Navigation => "NAVIGATION",
            Note => "NOTE",
            Notification => "NOTIFICATION",
            Option => "OPTION",
            PageTab => "PAGE_TAB",
            PageTabList => "PAGE_TAB_LIST",
            Paragraph => "PARAGRAPH",
            Panel => "PANEL",
            PopupButton => "POPUP_BUTTON",
            ProgressBar => "PROGRESS_BAR",
            RadioButton => "RADIO_BUTTON",
            RadioGroup => "RADIO_GROUP",
            Row => "ROW",
            RowGroup => "ROW_GROUP",
            RowHeader => "ROW_HEADER",
            ScrollBar => "SCROLL_BAR",
            ScrollPane => "SCROLL_PANE",
            Search => "SEARCH",
            Section => "SECTION",
            Separator => "SEPARATOR",
            Slider => "SLIDER",
            SpinButton => "SPIN_BUTTON",
            Splitter => "SPLITTER",
            StaticText => "STATIC_TEXT",
            StatusBar => "STATUS_BAR",
            Strong => "STRONG",
            Subscript => "SUBSCRIPT",
            Superscript => "SUPERSCRIPT",
            Suggestion => "SUGGESTION",
            Switch => "SWITCH",
            Table => "TABLE",
            Term => "TERM",
            Time => "TIME",
            Timer => "TIMER",
            TitleBar => "TITLE_BAR",
            ToolBar => "TOOL_BAR",
            ToolTip => "TOOL_TIP",
            Tree => "TREE",
            TreeItem => "TREE_ITEM",
            Unknown => "UNKNOWN",
            Window => "WINDOW",
        }
    }

    /// Lower-case role name used in the Playwright-MCP-style YAML snapshot
    /// (e.g. `button`, `entry_multiline`).
    pub fn yaml_name(&self) -> String {
        self.as_str().to_ascii_lowercase()
    }

    /// Single-character prefix for session ref aliases (e.g. `b` for buttons,
    /// `e` for editable/generic elements). Used by [`crate::AliasTable`] to mint
    /// short agent-facing aliases like `b1`, `e5`.
    pub fn alias_prefix(&self) -> char {
        use SlugRole::*;
        match self {
            Button | ButtonToggle | PopupButton | Switch => 'b',
            Link => 'l',
            Checkbox | RadioButton | MenuItemCheckbox | MenuItemRadio => 'c',
            Menu | MenuBar | MenuItem => 'm',
            Entry | EntryMultiline | EntryPassword | EntrySearch | ComboBox | SpinButton => 'i',
            Slider | ProgressBar | Meter => 's',
            Heading => 'h',
            // Everything else (generic elements, text, containers): 'e'.
            _ => 'e',
        }
    }

    /// Whether this role denotes a directly interactive control. Used by the
    /// coverage heuristic and YAML state synthesis.
    pub fn is_interactive(&self) -> bool {
        use SlugRole::*;
        matches!(
            self,
            Button
                | ButtonToggle
                | PopupButton
                | Switch
                | Link
                | Checkbox
                | RadioButton
                | MenuItem
                | MenuItemCheckbox
                | MenuItemRadio
                | Entry
                | EntryMultiline
                | EntryPassword
                | EntrySearch
                | ComboBox
                | SpinButton
                | Slider
                | PageTab
                | ListItem
                | TreeItem
                | Cell
                | GridCell
        )
    }

    /// Whether the agent should expect a screenshot/vision fallback for this role
    /// (CANVAS, IMAGE without a name, MEDIA) per §3.1 notes and axiom A1.
    pub fn is_opaque_surface(&self) -> bool {
        matches!(self, SlugRole::Canvas | SlugRole::Image | SlugRole::Media)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        for role in [
            SlugRole::Button,
            SlugRole::EntryMultiline,
            SlugRole::MenuItemCheckbox,
            SlugRole::Window,
        ] {
            let json = serde_json::to_string(&role).unwrap();
            let back: SlugRole = serde_json::from_str(&json).unwrap();
            assert_eq!(role, back);
        }
    }

    #[test]
    fn wire_form_is_screaming_snake() {
        assert_eq!(serde_json::to_string(&SlugRole::EntryMultiline).unwrap(), "\"ENTRY_MULTILINE\"");
        assert_eq!(serde_json::to_string(&SlugRole::Button).unwrap(), "\"BUTTON\"");
    }

    #[test]
    fn unknown_is_fallback_for_unrecognised() {
        let r: SlugRole = serde_json::from_str("\"SOMETHING_NEW\"").unwrap();
        assert_eq!(r, SlugRole::Unknown);
    }
}
