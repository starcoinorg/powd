function printResult(result, json) {
  if (json) {
    // eslint-disable-next-line no-console
    console.log(JSON.stringify(result.structuredContent, null, 2));
    return;
  }

  const textBlock = Array.isArray(result.content)
    ? result.content
        .map((block) => (block && block.type === "text" && typeof block.text === "string" ? block.text : null))
        .filter(Boolean)
        .join("\n")
    : "";
  // eslint-disable-next-line no-console
  console.log(textBlock);
}

export function registerPowdCli({ program, runInstall, runStatus }) {
  const powd = program.command("powd").description("Install or inspect the local powd setup");

  powd
    .command("status")
    .description("Show whether powd is installed and registered")
    .option("--version <version>", "compare the current install against a specific powd release")
    .option("--json", "print JSON")
    .action(async (options) => {
      const result = await runStatus(options.version);
      printResult(result, Boolean(options.json));
    });

  powd
    .command("install")
    .description("Install powd from GitHub Releases and register it with OpenClaw")
    .option("--version <version>", "install a specific powd release instead of the latest stable one")
    .option("--json", "print JSON")
    .action(async (options) => {
      const result = await runInstall(options.version);
      printResult(result, Boolean(options.json));
    });
}
