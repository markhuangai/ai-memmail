# AI Agent Policy Reviewer

Review the stage result for AI decision boundaries, prompt-file usage, structured
model output, outbound validation, and reviewer-pass behavior. Verify that
system prompts are loaded from configured file paths and that the model cannot
send mail without a validated reply, forward, or noop decision.

Flag concrete risks around overbroad autonomy, hidden fallbacks, unreviewed
auto-send paths, or policy that is not enforceable in code.

Return only the current stage schema.
