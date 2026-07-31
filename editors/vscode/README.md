# Stainless Language Support for Visual Studio Code

This extension adds syntax highlighting and basic editing support for Stainless
source files (`.stl`). It is declarative and does not run extension code.

Included support:

- Stainless keywords, declarations, primitive and ownership types
- Strings, characters, numeric literals, comments, macros, and operators
- Bracket matching, automatic closing, indentation, comment toggling, and
  `// region` folding
- Language id `stainless` for `.stl` files

Compiler diagnostics, completion, navigation, and formatting are not part of
this initial extension.

## Try it from the repository

From the repository root, open an Extension Development Host:

```sh
code --new-window --extensionDevelopmentPath="$(pwd)/editors/vscode"
```

Open a file under `docs/ref/` in that window. To inspect an individual token,
run **Developer: Inspect Editor Tokens and Scopes** from the command palette.

## Package it

With Node.js installed, package the extension from this directory:

```sh
npx @vscode/vsce package
```

Install the generated VSIX with:

```sh
code --install-extension stainless-language-0.1.0.vsix
```

The grammar should stay aligned with
`crates/stainless-syntax/src/lexer.rs` whenever the language changes.
