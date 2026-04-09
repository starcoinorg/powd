export function buildApprovalRequest(status, version) {
  const lines = [
    `Download powd ${version} from GitHub Releases, install it locally, and register it with OpenClaw.`,
    "This will not set a wallet or start mining.",
  ];
  if (status.foreignRegistration) {
    lines.push("An existing powd MCP registration points somewhere else and will be replaced.");
  }
  return {
    title: "Install powd",
    description: lines.join(" "),
    severity: "warning",
    timeoutBehavior: "deny",
    timeoutMs: 5 * 60 * 1000,
  };
}

function formatStatusLines(status) {
  return [
    `installed: ${status.installed ? "yes" : "no"}`,
    `registered: ${status.registered ? "yes" : "no"}`,
    `version: ${status.version ?? "(unknown)"}`,
    `binary path: ${status.binaryPath ?? "(not installed)"}`,
    `registration matches install: ${status.mcpCommandMatchesInstall ? "yes" : "no"}`,
    `platform supported: ${status.platformSupported ? "yes" : "no"}`,
    "",
    status.message,
  ];
}

export function buildStatusToolResult(status) {
  return {
    content: [{ type: "text", text: formatStatusLines(status).join("\n") }],
    structuredContent: status,
  };
}

export function buildInstallToolResult(result) {
  return {
    content: [{ type: "text", text: result.message }],
    structuredContent: result.status,
  };
}

export function buildStatusCommandReply(status) {
  return {
    text: formatStatusLines(status).join("\n"),
  };
}

export function buildInstallCommandReply(result) {
  return {
    text: result.message,
  };
}

