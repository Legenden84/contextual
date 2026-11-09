# ContextBar

Context budget above the input: files, rules, estimated input tokens, a meter against the deployment's input limit, and the estimated cost per message.

Alongside the meter, show the effective configured completion limit as `Completion limit: 16,384 tokens / response`, with `(includes reasoning where applicable)`. This is a separate per-response allowance, not part of the input percentage or a live output-usage meter. Hide the completion allowance when Foundry has no valid configuration.

Under 75% the meter is `accent`; 75–94% turns `warning` with "Near limit" and a hint about the biggest contributor; 95% and above turns `danger` with "Over limit". The cost tooltip shows the prices from Settings.
