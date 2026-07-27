const [, , workspace, entrypoint, ...args] = process.argv;

if (!workspace?.startsWith("/") || !entrypoint?.startsWith("/")) {
  throw new Error("Cursor ACP bootstrap requires absolute workspace and entrypoint paths");
}

process.chdir(workspace);
process.cwd = () => workspace;
process.argv = [process.argv[0], entrypoint, ...args];
require(entrypoint);
