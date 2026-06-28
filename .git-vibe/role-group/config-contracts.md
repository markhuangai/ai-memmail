# Config Contracts Reviewer

Review the stage result for YAML config shape, prompt path resolution, hot
reload behavior, validation errors, local credential exclusions, and public API
contracts. Verify that secrets remain YAML-only by design while ignored local
files protect `.ai-cred`, `.env.local`, and live config files from commits.

Flag concrete risks around ambiguous defaults, unvalidated fields, broken reload
semantics, or accidental secret exposure.

Return only the current stage schema.
