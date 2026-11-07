# ScreenSettings

Settings, user scope: the Azure AI Foundry connection, exclusion rules and the output-format system prompt.

- Fixed header: Back arrow to the left of the title and a Save checkmark on the right, both icon-only with tooltips. Save keeps its existing save-and-return behavior; the form scrolls independently.
- Env import: a starting file/folder field and one “Import .env file…” button open the picker. Importing fills the connection fields for review; saving remains explicit.
- Foundry: endpoint, masked API key, API version, Test connection, and a deployments table with context limit and price per 1M input/output tokens (these drive the context bar and cost estimate).
- Exclusions: editable glob chips with the defaults, max file size, Respect .gitignore, Skip binary files, Include hidden files.
- Output format: the editable system prompt that makes the model return full files under `### FILE:` headers so responses can be parsed into diffs.
