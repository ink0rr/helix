(type) @type
(number) @constant.numeric

; `@param foo`, `@param foo.bar`, `@param [foo=1]`, `@property [foo.bar]`
(tag [(identifier) (member_expression) (path_expression) (qualified_expression)] @variable.parameter)
(optional_identifier (identifier) @variable.parameter)
(optional_identifier "=" @operator)
(optional_identifier value: (_) @variable.parameter)
(array_expression (member_expression) @variable.parameter)
; `[foo.bar=1]`: the grammar can't parse a dotted name with a default, so the `=1` is an ERROR.
(array_expression (ERROR "=" @operator))
(array_expression (ERROR (_) @variable.parameter))

; Names that declare a type: `@typedef {Object} Foo`, `@callback Cb`
(tag
  (tag_name) @_tag
  (identifier) @type
  (#any-of? @_tag "@typedef" "@callback"))

; Last, so it wins over the `@_tag` capture above.
(tag_name) @keyword
