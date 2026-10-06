# cdev for VSCodium / VS Code

A thin client over the `cdev` sidecar (install it first: `cargo install --path .` in the repo root).

## Features

- **Auto-watch this file**: the 👁 button in the editor title bar, or the `▶ auto-watch <fn>` CodeLens above each function (Python, JS, TS). It runs `cdev --auto watch <file>`, which re-runs on save.
- **Inline values**: the latest values at the end of each line. Loop steps show what changed (`i=1  j=0`).
- **Hover**: a recorded line shows its type, data structure, full value and earlier values.
- **Panel**: the full cdev panel (Live · Vars · API · Map · Mem) opens beside your code.
  - Clicking a `file:line` location opens it.
  - Stepping in **Vars** / **Mem** (← → / play) highlights the line that caused each step.
- **Static view** (JS, TS; nothing runs and no sidecar is needed): a CodeLens above each function says what it calls, what calls it and what it changes (`calls 2 · called by main · changes its input`). Hovering the function's first line gives the details, and **Explain this file** (title bar, or click a lens) opens the whole report beside the code.
- **Run a command under cdev…**, e.g. `npm run dev`.
- **Attach to a running cdev**: attaching is automatic if one is already running on the configured port.

Settings: `cdev.path`, `cdev.port`, `cdev.inlineValues`, `cdev.codeLens`, `cdev.staticLens`.

## Install (local)

```sh
# from the repo root
cd editor/vscode
npx @vscode/vsce package --no-dependencies     # → cdev-0.1.0.vsix
codium --install-extension cdev-0.1.0.vsix     # VSCodium
code   --install-extension cdev-0.1.0.vsix     # VS Code
```

To develop the extension itself, run `codium --extensionDevelopmentPath=$PWD`.
