# ContextBar

Context budget above the input: files, rules, estimated input tokens, a meter against the deployment's input limit, and the estimated cost per message.

Keep this summary focused on input context; do not show a completion-limit label or reasoning disclaimer. The configured output limit still applies to each API request, independently of how many rounds a user message takes.

Under 75% the meter is `accent`; 75–94% turns `warning` with "Near limit" and a hint about the biggest contributor; 95% and above turns `danger` with "Over limit". The cost tooltip shows the prices from Settings.
