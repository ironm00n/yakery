; (id_expr) @variable ;; fallback


["end"] @keyword

[
 "is"
 "is=="
 "is=~"
 "is<=>"
 "is-roughly"
 "is-not-roughly"
 "is-not"
 "is-not=="
 "is-not=~"
 "is-not<=>"
 "raises"
 "raises-other-than"
 "satisfies"
 "violates"
 "raises-satisfies"
 "raises-violates"
] @keyword.operator

[
  "if"
  "else:"
  "else if"
  "ask"
  "when"
  "then:"
  "otherwise:"
  "cases"
  "else"
] @keyword.conditional

;; end should match the keyword it was opened with
(cases_expr "end" @keyword.conditional)
(if_expr "end" @keyword.conditional)
(when_expr "end" @keyword.conditional)
(if_pipe_expr "end" @keyword.conditional)

[
  "import"
  "include"
  "provide"
  "provide:"
  "provide-types"
] @keyword.import

(provide_vals_stmt "end" @keyword.import)
(provide_block "end" @keyword.import)
(import_stmt "from" @keyword.import "end" @keyword.import)

(import_stmt
  "as" @keyword.import
  (name) @module)
(import_stmt
  "from" @keyword.import)

[
  "for"
] @keyword.repeat
(for_expr "end" @keyword.repeat)


[
  "fun"
  "var"
  "shadow"
  "let"
  "doc:"
  "from"
  "load-table"
  "lam"
  "block:"
  "type"
  "method"
  "data"
  "sharing:"
  "with:"
  "check"
  "check:"
  "where:"
  "spy"
  "table:"
  "row:"
] @keyword
[
  "and"
  "or"
] @keyword
[
  "::"
  ":"
  "."
  "!"
  "->"
  "=>"
  ","
] @punctuation.delimiter

[
  "+"
  "=="
  "|"
  "="
  ":="
 (template_expr)
] @operator

[
  (line_comment)
  (block_comment)
] @comment @spell

(ann_field) @type
(name_ann) @type
(dot_ann
  _
  (name) @type)
(string) @string @spell
(escape_sequence) @string.escape
(bool_expr) @constant.builtin


[
  (num_expr)
  (frac_expr)
  (rfrac_expr)
] @number

(fun_expr (name) @function)
(field (key) @variable.member)
(field (key) @function.method (fun_header))
(obj_field (key) @variable.member)
(obj_field (key) @function.method (fun_header))

(ty_params (comma_names (name) @type))
(type_expr (name) @type)

(data_expr (name) @type.definition)
(variant_constructor (name) @constructor)
(data_variant (name) @constructor) ; this is kinda a lie
(first_data_variant (name) @constructor) ; ditto

;; TODO: add dot expr's etc

(app_expr (id_expr) @function.call)
(app_expr (dot_expr (name) @function.call))

; TODO: idk if this is correct
(variant_member (binding (name_binding (name) @variable.member)))

((args (binding (name_binding (name) @variable.parameter))))
((args (binding (tuple_binding (binding (name_binding (name) @variable.parameter))))))

(cases_branch (name) @constructor)
(cases_binding (binding (name_binding (name) @definition.variable)))
(cases_binding (binding (tuple_binding (binding (name_binding (name) @definition.variable)))))

(for_expr (id_expr (id_expr)) @function.call)
(for_bind (binding (name_binding (name) @definition.variable)))
(for_bind (binding (tuple_binding (binding (name_binding (name) @definition.variable)))))

(let_expr (toplevel_binding (binding (name_binding (name) @variable))))
(let_expr (toplevel_binding (binding (tuple_binding (binding (name_binding (name) @variable))))))


