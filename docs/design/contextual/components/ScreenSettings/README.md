# ScreenSettings

Settings, user scope: the Azure AI Foundry connection, exclusion rules and the output-format system prompt.

- Fixed header: Back arrow to the left of the title and a Save checkmark on the right, both icon-only with tooltips. Save keeps its existing save-and-return behavior and saves edits across all groups.
- Persistent left navigation: Azure AI Foundry (initial selection), Exclusions, Output format and Appearance. Only the selected group appears in the right-hand form, which scrolls independently. Switching groups preserves unsaved values and does not save anything.
- Navigation and form share one centered, width-limited layout beneath the header. Extra window width becomes outer margins, keeping navigation beside the form rather than pinned to the window edge.
- Navigation uses the sidebar surface and a hairline divider, with compact control-height text rows. The selected row uses the neutral editor surface, strong ink and an inset 2px accent line on the left, echoing the app's active-tab indicator without a coloured selection fill or chevron. Inactive rows have a subtle hover fill; keyboard focus has a separate visible outline. Tab focuses each entry, Enter or Space selects it, Up/Down selects the previous/next group, and Home/End selects the first/last group.
- Save errors and import feedback stay visible below the header regardless of the selected group. On narrow windows, the form scrolls horizontally without scrolling the navigation away.
- Env import: a starting file/folder field and one “Import .env file…” button open the picker. Importing fills the connection fields for review; saving remains explicit.
- Foundry: endpoint, masked API key, API version, Test connection, and a deployments table with context limit and price per 1M input/output tokens (these drive the context bar and cost estimate).
- Exclusions: editable glob chips with the defaults, max file size, Respect .gitignore, Skip binary files, Include hidden files.
- Output format: the editable system prompt that makes the model return full files under `### FILE:` headers so responses can be parsed into diffs.
- Appearance: Dark, Light and Moss schemes, plus the existing System preference. Selection applies and saves immediately. The title-bar toggle cycles only the three explicit schemes; changing it while Settings is open updates the selected option without discarding form edits or letting Save restore an older scheme.
- Reduce motion: an explicit app preference under Appearance, saved with the form. Activity marks become static while their labels continue to report work. Tab focuses the control; Enter or Space toggles it. This preference does not automatically detect the OS motion setting.
