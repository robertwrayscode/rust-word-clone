# RWord

A word processor written in Rust, in the style of Microsoft Word, for Windows and macOS.

## Download

Get the latest version from the [Releases page](https://github.com/robertwrayscode/rust-word-clone/releases/latest):

| Computer | File |
|---|---|
| Mac with Apple Silicon (M1, M2, M3, M4...) | `RWord-Mac-AppleSilicon.zip` |
| Mac with an Intel processor | `RWord-Mac-Intel.zip` |
| Windows 10 / 11 | `RWord-Windows.zip` |

**Mac:** unzip, drag `RWord.app` to Applications, then right-click it and choose **Open** the first time
(or System Settings > Privacy & Security > **Open Anyway**), because the app is not signed by Apple.

**Windows:** unzip and double-click `RWord.exe`. If SmartScreen appears, click **More info > Run anyway**.

## Features

- Pages laid out like Word: margins, headers and footers, page numbers, columns, page breaks, watermarks, page colour and borders
- Character formatting (fonts, sizes, bold, italic, underline styles, colours, highlight, sub/superscript, caps), paragraph formatting (alignment, indents, spacing, tabs, borders, shading)
- Styles (Normal, Headings, Title, Quote...), bullet and numbered lists, multilevel lists
- Tables with merged cells, header rows, shading, borders and table styles
- Pictures, hyperlinks, bookmarks, footnotes and endnotes, captions, table of contents, symbols, date and time
- Comments and Track Changes (accept / reject)
- Spell check with suggestions, grammar hints, AutoCorrect, thesaurus, word count
- Find and Replace, Go To, Navigation pane, zoom, ruler, formatting marks
- Opens and saves Word (.docx), OpenDocument (.odt), Rich Text (.rtf), web pages (.html), plain text and Markdown; exports PDF and prints

## Building

```
cargo run -p app --release            # start RWord
cargo run -p app -- --demo            # start with a sample document
cargo test -p wordcore                # run the engine tests
```

Linux builds need the usual GUI development packages (`libxkbcommon-dev libgl1-mesa-dev libgtk-3-dev`, etc.).

## Licence

MIT. Bundled fonts are under the SIL Open Font License (see `assets/fonts/licenses`).
Proofing data licences are in `crates/core/assets/proofing/licenses`.
