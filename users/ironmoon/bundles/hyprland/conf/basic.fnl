(import-macros hl! :lib.checked)

(local nix (require! :nix))

(hl.on :hyprland.start
  #(each [_ mime (ipairs [:text :image])]
    (hl.exec_cmd (.. "wl-paste --type " mime " --watch cliphist -max-items 100000 store"))))

(hl.env :XCURSOR_SIZE "24")
(hl.env :HYPRCURSOR_SIZE "24")
(hl.env :NIXOS_OZONE_WL "1")
(when nix.nvidia
  (hl.env :LIBVA_DRIVER_NAME :nvidia)
  (hl.env :__GLX_VENDOR_LIBRARY_NAME :nvidia))

(hl!.config {
  :general {
    :gaps_in 0
    :gaps_out 0
    :border_size 1
    :col {
      :active_border "rgba(33ccffee)"
      :inactive_border "rgba(595959aa)"
    }
    :resize_on_border true
    :extend_border_grab_area 20
    :layout :dwindle
  }
  :decoration {
    :rounding 0
    :shadow {:enabled false}
    :blur {:enabled false}
  }
  :animations {:enabled false}
  :input {
    :touchpad {
      :clickfinger_behavior true
      :natural_scroll true
    }
  }
  ;; TODO: https://wiki.hyprland.org/Configuring/Dwindle-Layout/
  :dwindle {:preserve_split true}
  ;; https://wiki.hyprland.org/Configuring/Variables/#misc
  :misc {
    :disable_hyprland_logo true ; default wallpaper
    :disable_splash_rendering true ; splash text
    :focus_on_activate true
    ;; :disable_autoreload true
    :middle_click_paste false
  }
})

(hl!.gesture {
  :fingers 3
  :direction :horizontal
  :action :workspace
})
;; FIXME: causing crashes
; (hl.gesture {:fingers 3 :direction :vertical :action :expo})

(hl.window_rule {
  :name :suppress-maximize-events
  :match {:class ".*"}
  :suppress_event :maximize
})

(hl.window_rule {
  :name :fix-xwayland-drags
  :match {
    :class "^$"
    :title "^$"
    :xwayland true
    :float true
    :fullscreen true
    :pin false
  }
  :no_focus true
})

;; TODO?: make xdg-portals float
;; TODO: mess with rules for waydroid

(hl.window_rule {
  :name :zoom-menu-window
  :match {
    :class "^(Zoom)$"
    :title "^(menu window)$"
  }
  :stay_focused true
})

(hl.window_rule {
  :name :zoom-confirm-window
  :match {
    :class "^(Zoom)$"
    :title "^(confirm window)$"
  }
  :stay_focused true
})

(hl.window_rule {
  :name :zoom-share-toolbar
  :match {
    :class "^(Zoom)$"
    :initial_title "^(as_toolbar)$"
  }
  :float true
  :pin true
  :no_anim true
})

(hl.layer_rule {
  :match {:namespace :anyrun}
  :dim_around true
})
