## Using pickers

Helix has a variety of pickers, which are interactive windows used to select various kinds of items. These include a file picker, global search picker, and more. Most pickers are accessed via keybindings in [space mode](./keymap.md#space-mode). Pickers have their own [keymap](./keymap.md#picker) for navigation.

### Filtering Picker Results

Most pickers perform fuzzy matching using [fzf syntax](https://github.com/junegunn/fzf?tab=readme-ov-file#search-syntax). Two exceptions are the global search picker, which uses regex, and the workspace symbol picker, which passes search terms to the language server. Note that OR operations (`|`) are not currently supported.

If a picker shows multiple columns, you may apply the filter to a specific column by prefixing the column name with `%`. Column names can be shortened to any prefix, so `%p`, `%pa` or `%pat` all mean the same as `%path`. For example, a query of `helix %p .toml !lang` in the global search picker searches for the term "helix" within files with paths ending in ".toml" but not including "lang".

You can insert the contents of a [register](./registers.md) using `Ctrl-r` followed by a register name. For example, one could insert the currently selected text using `Ctrl-r`-`.`, or the directory of the current file using `Ctrl-r`-`%` followed by `Ctrl-w` to remove the last path section. The global search picker will use the contents of the [search register](./registers.md#default-registers) if you press `Enter` without typing a filter. For example, pressing `*`-`Space-/`-`Enter` will start a global search for the currently selected text.

### File explorer

`Space-e` opens a file explorer for the workspace root; `-` opens one for the current buffer's directory, or the parent directory when used in an explorer. Like [oil.nvim](https://github.com/stevearc/oil.nvim), the explorer is a buffer listing the directory's entries, one per line, with directories ending in `/`. In normal mode, `Enter` opens the entry under the cursor, unless it is bound in your keymap.

Edit the listing like any text and `:write` it to apply the changes to the file system: editing a line renames (or moves, when the new name contains a `/`) its entry, deleting a line deletes its entry, and adding a line creates a file, or a directory if it ends with `/`. Explorer buffers with unsaved edits stay open while you browse other directories and are all written together, so cutting a line in one directory and pasting it in another moves the entry. Deletions ask for confirmation first, and `:reload` discards the edits. Unlike the file picker, the explorer does not ignore most files by default; its ignore behaviour is configured separately in the [`[editor.file-explorer]`](./editor.md#editorfile-explorer-section) section.
