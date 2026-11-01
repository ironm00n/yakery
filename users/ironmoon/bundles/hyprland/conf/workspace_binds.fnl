(import-macros {: bind} :lib.macros)

;; Switch workspaces with SUPER (ALT)?, [1-9]|F[1-10]
;; Move active window to a workspace with SUPER (ALT)?, [1-9]|F[1-10]
(for [i 1 20]
  (let [key (if (<= i 10) (tostring (% i 10)) (.. :F (- i 10)))]
    (each [alt ws (pairs {"" i " + ALT" (+ i 20)})]
      (bind (.. "SUPER" alt " + " key) (hl.dsp.focus {:workspace ws}))
      (bind (.. "SUPER + SHIFT" alt " + " key) (hl.dsp.window.move {:workspace ws})))))
