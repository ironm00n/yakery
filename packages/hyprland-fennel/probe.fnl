;; a hyprland config, run by `Hyprland --verify-config` against this very build: it asks the
;; runtime which keys each spec-taking function reads and writes them out as hl-keys.fnl
(local fennel (require :fennel))
(local out (os.getenv "HL_KEYS_OUT"))
(local stubs (os.getenv "HL_STUBS"))

;; per function: how to call it with a spec, the macro argument holding the spec, and the
;; smallest spec the runtime accepts
(local calls {
  :gesture {
    :path :hl.gesture
    :arg 1
    :call #(hl.gesture $)
    :example {:fingers 3 :direction :horizontal :action :workspace}
  }
  :bind {
    :path :hl.bind
    :arg 3
    :call #(hl.bind "SUPER + F24" (hl.dsp.exec_cmd "true") $)
    :example {}
  }
  :device {
    :path :hl.device
    :arg 1
    :call #(hl.device $)
    :example {:name :probe}
  }
  :monitor {
    :path :hl.monitor
    :arg 1
    :call #(hl.monitor $)
    :example {:output :probe :mode :preferred :position :auto :scale 1}
  }
  :permission {
    :path :hl.permission
    :arg 1
    :call #(hl.permission $)
    :example {:binary "/probe" :type :screencopy :mode :allow}
  }
  :window_rule {
    :path :hl.window_rule
    :arg 1
    :call #(hl.window_rule $)
    :example {:name :probe :match {:class :probe}}
  }
  :layer_rule {
    :path :hl.layer_rule
    :arg 1
    :call #(hl.layer_rule $)
    :example {:match {:namespace :probe}}
  }
  :workspace_rule {
    :path :hl.workspace_rule
    :arg 1
    :call #(hl.workspace_rule $)
    :example {:workspace "1"}
  }
})

;; runs twice, since the runtime records spec errors instead of raising them. classify: every
;; spec also carries a raw key the runtime never looks up, so the functions that iterate their
;; spec report it as an unknown field, and the caller collects those into HL_ITERATING. those
;; already reject unknown keys themselves; for the rest, the keys they look up are all there is.
(local classify? (= (os.getenv "HL_PROBE") :classify))
(local iterating
  (collect [name (string.gmatch (or (os.getenv "HL_ITERATING") "") "%S+")]
    name true))

(fn probe [{: call : example}]
  (let [read {}]
    (call (setmetatable (if classify? {:__probe_unknown true} {}) {
      :__index (fn [_ k]
        (tset read k true)
        (. example k))
    }))
    read))

(when classify?
  (each [_ spec (pairs calls)]
    (probe spec))
  (lua "do return end"))

(assert (next iterating) "HL_ITERATING is empty; did the classify run's output format change?")

(local checked
  (collect [name spec (pairs calls)]
    (if (not (. iterating name))
      (values name {:path spec.path :arg spec.arg :keys (probe spec)}))))

(local config-keys
  (let [keys {}]
    (var in-alias false)
    (each [line (io.lines stubs)]
      (if (= line "---@alias HL.ConfigKey")
        (set in-alias true)
        in-alias
        (case (line:match "^%-%-%-| \"(.+)\"$")
          key (tset keys key true)
          _ (set in-alias false))))
    keys))

(local unknown-config-keys
  (icollect [key (pairs config-keys)]
    (let [(ok _ err) (pcall hl.get_config key)]
      (if (or (not ok) err) key))))

(assert (next config-keys) (.. stubs ": no HL.ConfigKey alias"))
(assert (= 0 (length unknown-config-keys))
  (.. "stub config keys the runtime does not know: " (table.concat unknown-config-keys " ")))

(with-open [f (assert (io.open (assert out "HL_KEYS_OUT unset") :w))]
  (f:write (fennel.view {:calls checked :config config-keys}) "\n"))
