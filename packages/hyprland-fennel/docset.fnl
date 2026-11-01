;; turns hyprland's LuaLS stubs into a fennel-ls docset for the `hl` global
(local [stubs] arg)

(local classes {})
(var class nil)
(var params [])

(fn split-top-level [s]
  (let [parts []]
    (var depth 0)
    (var start 1)
    (for [i 1 (length s)]
      (case (s:sub i i)
        (where c (c:match "[%(%[{<]")) (set depth (+ depth 1))
        (where c (c:match "[%)%]}>]")) (set depth (- depth 1))
        (where "," (= depth 0))
        (do
          (table.insert parts (s:sub start (- i 1)))
          (set start (+ i 1)))))
    (table.insert parts (s:sub start))
    (icollect [_ part (ipairs parts)]
      (let [part (part:match "^%s*(.-)%s*$")]
        (if (not= part "") part)))))

(fn arglist [ty]
  (case (ty:match "^fun(%b())")
    params
    (icollect [_ param (ipairs (split-top-level (params:sub 2 -2)))]
      (if (= param "...")
        param
        (let [(name optional) (param:match "^([%w_]+)(%??)")]
          (.. optional name))))))

(each [line (io.lines stubs)]
  (let [class-name (line:match "^%-%-%-@class ([%w_.]+)")
        (field field-type) (line:match "^%-%-%-@field ([%w_]+)%??%s+(.+)$")
        (param param-type) (line:match "^%-%-%-@param ([%w_]+)%s+(.+)$")
        method (line:match "^function __HL_API%.([%w_]+)%(")]
    (if class-name
      (do
        (set class [])
        (tset classes class-name class))
      (and field class)
      (table.insert class {:name field :type field-type})
      param
      (table.insert params (.. param ": " param-type))
      method
      (do
        (table.insert (. classes :HL.API) {
          :name method
          :type (.. "fun(" (table.concat params ", ") ")")
        })
        (set params []))
      (line:match "^local ")
      (set class nil))))

(fn entry [binding ty]
  (let [fields (. classes ty)]
    (if fields
      {
        :binding binding
        :metadata {:fls/itemKind :Module :fnl/docstring ty}
        :fields (collect [_ field (ipairs fields)]
          field.name (entry (.. binding "." field.name) field.type))
      }
      (arglist ty)
      {
        :binding binding
        :metadata {
          :fls/itemKind :Function
          :fnl/arglist (arglist ty)
          :fnl/docstring ty
        }
      }
      {
        :binding binding
        :metadata {:fls/itemKind :Variable :fnl/docstring ty}
      })))

(fn lua-literal [x]
  (case (type x)
    :string (string.format "%q" x)
    (where (or :number :boolean)) (tostring x)
    :table
    (let [keys (icollect [k (pairs x)] k)]
      (table.sort keys #(< (tostring $1) (tostring $2)))
      (.. "{" (table.concat (icollect [_ k (ipairs keys)] (.. "[" (lua-literal k) "]=" (lua-literal (. x k)))) ",") "}"))))

(assert (. classes :HL.API) (.. stubs ": no HL.API class"))
(print (.. "return " (lua-literal {:hl (entry :hl :HL.API)})))
