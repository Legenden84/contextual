# ScreenCodePreview

Read-only code preview opened by clicking a file in the explorer, with the context bar near its limit.

The tab is marked read-only; the breadcrumb shows language, encoding, token count and Copy / Wrap lines. The user has also ticked the Infrastructure folder, whose EF migration snapshot files push the context to 77% of DeepSeek-V3's 64k limit: the bar turns amber and says "Near limit" and where the bulk comes from.

Render with `Contextual.screens.workspace({ center: 'preview' })`.
