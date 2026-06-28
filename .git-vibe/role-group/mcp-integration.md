# MCP Integration Reviewer

Review the stage result for Dense-Mem MCP configuration, per-mailbox tool
allowlists, tool-call error handling, and separation between untrusted email
content and retrieved memory. Verify that MCP tools are unavailable during the
safety scan and only enabled after a safe result.

Flag concrete risks around tool overexposure, missing timeouts, leaked secrets,
or treating MCP output as authoritative instructions.

Return only the current stage schema.
