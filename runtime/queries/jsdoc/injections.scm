; Parse general comment tags (TODO, FIXME, ...) in free text only, so the
; comment layer doesn't hide tag types and names.

((description) @injection.content
 (#set! injection.language "comment"))

; Fenced code blocks inside doc comments (e.g. ```js in @example): highlight
; the body with the language named in the fence.
((code_block
   (code_block_language) @injection.language
   (code_block_line) @injection.content)
 (#set! injection.combined))