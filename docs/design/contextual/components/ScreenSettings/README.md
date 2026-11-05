# ScreenSettings

Settings, user scope: the Azure AI Foundry connection, exclusion rules and the output-format system prompt.

- Foundry: endpoint, masked API key, API version, Test connection, and a deployments table with context limit and price per 1M input/output tokens (these drive the context bar and cost estimate).
- Exclusions: editable glob chips with the defaults, max file size, Respect .gitignore, Skip binary files, Include hidden files.
- Output format: the editable system prompt that makes the model return full files under `### FILE:` headers so responses can be parsed into diffs.
