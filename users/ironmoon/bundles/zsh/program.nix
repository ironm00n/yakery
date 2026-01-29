{
  lib,
  config,
  pkgs,
}:
let
  inherit (lib)
    mkMerge
    mkAfter
    mkOrder
    getExe
    ;
  inherit (lib.strings) optionalString;
in
{
  enable = true;
  defaultKeymap = "emacs";
  dotDir = "${config.xdg.configHome}/zsh";

  shellAliases = {
    ll = "ls -l";
    la = "ls -lAh";
    l = "ls -lah";
    diff = "diff -u";
    vim = "nvim";
    lg = "lazygit";

    kssh = "kitten ssh";
    kdiff = "kitten diff";
    icat = "kitten icat";
  };
  history = {
    size = 1000000;
    path = "${config.xdg.dataHome}/zsh/history";
  };
  syntaxHighlighting = {
    enable = true;
  };
  autosuggestion = {
    enable = true;
  };
  historySubstringSearch = {
    enable = true;
    searchUpKey = [
      "^[[A"
      "^[OA"
    ];
    searchDownKey = [
      "^[[B"
      "^[OB"
    ];
  };
  enableCompletion = true;
  completionInit = /* zsh */ ''
    autoload -zU compinit
    _global_compinit
  '';
  initContent = mkMerge [
    # When in a TTY we should be careful, ZSH is our login shell its best we fall back to
    # safer operations
    (mkOrder 480 /* zsh */ ''
      case $(tty) in
        (/dev/tty[1-9]) IS_TTY=1;;
                    (*) IS_TTY=0;;
      esac
    '')
    # Here we create a hook to ensure zsh completions are properly enabled for applications
    # only loaded in a direnv enviroment.
    # In `mkShell`, pkgs provided to `nativeBuildInputs` have their `/share` added to
    # `$XDG_DATA_DIRS`, which we can add to `fpath` if applicable
    # TODO(2025-12-05): remove if https://github.com/direnv/direnv/issues/443 is resolved
    (mkOrder 490 /* zsh */ ''
      typeset -U fpath
      typeset -g _DIRENV_COMPINIT_KEY
      typeset -ga _GLOBAL_FPATH

      _global_compinit() {
        compinit -u -d "$ZSH_CACHE_DIR/zcompdump"
      }

      _direnv_compinit() {
        if [[ -z "$DIRENV_DIR" ]]; then
          if [[ -n "$_DIRENV_COMPINIT_KEY" ]]; then
            local cmd func
            for func in "''${(@v)_comps}"; do
              unfunction "$func" 2>/dev/null
            done

            fpath=("''${_GLOBAL_FPATH[@]}")
            _global_compinit
            unset _DIRENV_COMPINIT_KEY
          fi
          return
        fi
        local key="$DIRENV_DIR|$XDG_DATA_DIRS"
        [[ "$key" == "$_DIRENV_COMPINIT_KEY" ]] && return
        _DIRENV_COMPINIT_KEY="$key"

        fpath=("''${_GLOBAL_FPATH[@]}")
        local d
        for d in ''${(s/:/)XDG_DATA_DIRS}; do
          [[ -d "$d/zsh/site-functions" ]]         && fpath+=("$d/zsh/site-functions")
          [[ -d "$d/zsh/$ZSH_VERSION/functions" ]] && fpath+=("$d/zsh/$ZSH_VERSION/functions")
          [[ -d "$d/zsh/vendor-completions" ]]     && fpath+=("$d/zsh/vendor-completions")
        done
        local hash=$(print -r -- "$key" | shasum | cut -d' ' -f1)
        compinit -u -d "$ZSH_CACHE_DIR/zcompdump-direnv-$hash"
      }
    '')
    (mkOrder 501 /* zsh */ ''
      ZSH_CACHE_DIR="''${XDG_CACHE_HOME:-$HOME/.cache}/zsh"
      mkdir -p "$ZSH_CACHE_DIR"
    '')
    # Ensure that the Powerlevel10k instant prompt is enabled before doing anything time consuming.
    # Additionally, we need to be careful with its interaction with direnv see:
    # https://github.com/romkatv/powerlevel10k/issues/702#issuecomment-626222730
    # The direnv completion support is only enabled in non-TTYs
    (mkOrder 500 /* zsh */ ''
      if ! (($IS_TTY)); then
        emulate zsh -c "$(${getExe pkgs.direnv} export zsh)"

        if [[ -r "''${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-''${(%):-%n}.zsh" ]]; then
          source "''${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-''${(%):-%n}.zsh"
        fi

        emulate zsh -c "$(${getExe pkgs.direnv} hook zsh)"
        # NOTE: `precmd_functions` and `chpwd_functions` are defined by executing `direnv hook zsh`
        typeset -ga precmd_functions chpwd_functions
        precmd_functions+=(_direnv_compinit)
        chpwd_functions+=(_direnv_compinit)
      else
        eval "$(${getExe pkgs.direnv} hook zsh)"
      fi
    '')
    # this is placed directly after populating fpath from NIX_PROFILES
    (mkOrder 521 /* zsh */ ''
      _GLOBAL_FPATH=("''${fpath[@]}")
    '')
    (mkOrder 550 /* zsh */ ''
      if ! (($IS_TTY)); then
        source ${pkgs.zsh-powerlevel10k}/share/zsh-powerlevel10k/powerlevel10k.zsh-theme
      fi
    '')
    /* zsh */ ''
      if ! (($IS_TTY)); then
        [[ -f ~/.p10k.zsh ]] && source ~/.p10k.zsh
      fi

      bindkey '^[[3~' delete-char         # del
      bindkey '^H' backward-kill-word     # ctrl + backspace
      bindkey '^[[3;5~' kill-word         # ctrl + del
      bindkey '^[[1;5D' backward-word     # ctrl + left
      bindkey '^[[1;5C' forward-word      # ctrl + right

      bindkey '^[[Z' reverse-menu-complete

      # stolen from oh-my-zsh
      WORDCHARS='''

      unsetopt menu_complete   # do not autoselect the first completion entry
      unsetopt flowcontrol
      setopt auto_menu         # show completion menu on successive tab press
      setopt complete_in_word
      setopt always_to_end

      zstyle ':completion:*:*:*:*:*' menu select
      zstyle ':completion:*' matcher-list 'm:{[:lower:][:upper:]-_}={[:upper:][:lower:]_-}' 'r:|=*' 'l:|=* r:|=*'
      zstyle ':completion:*' special-dirs true
      zstyle ':completion:*' list-colors "''${(s.:.)LS_COLORS}"
      zstyle ':completion:*:*:kill:*:processes' list-colors '=(#b) #([0-9]#) ([0-9a-z-]#)*=01;34=0=01'
      zstyle ':completion:*:*:*:*:processes' command "ps -u $USERNAME -o pid,user,comm -w -w"
      zstyle ':completion:*:cd:*' tag-order local-directories directory-stack path-directories
      zstyle ':completion:*' use-cache yes
      zstyle ':completion:*' cache-path $ZSH_CACHE_DIR
      zstyle ':completion:*:*:*:users' ignored-patterns \
        adm amanda apache at avahi avahi-autoipd beaglidx bin cacti canna \
        clamav daemon dbus distcache dnsmasq dovecot fax ftp games gdm \
        gkrellmd gopher hacluster haldaemon halt hsqldb ident junkbust kdm \
        ldap lp mail mailman mailnull man messagebus mldonkey mysql nagios \
        named netdump news nfsnobody nobody nscd ntp nut nx obsrun openvpn \
        operator pcap polkitd postfix postgres privoxy pulse pvm quagga radvd \
        rpc rpcuser rpm rtkit scard shutdown squid sshd statd svn sync tftp \
        usbmux uucp vcsa wwwrun xfs '_*'
      zstyle '*' single-ignored show
      # end stolen

      man() {
        GROFF_NO_SGR=1 \
        LESS_TERMCAP_ue=$'\e[00m' \
        LESS_TERMCAP_se=$'\e[00m' \
        LESS_TERMCAP_mb=$'\e[01;31m' \
        LESS_TERMCAP_md=$'\e[01;31m' \
        LESS_TERMCAP_me=$'\e[00m' \
        LESS_TERMCAP_so=$'\e[01;33m\e[44m' \
        LESS_TERMCAP_us=$'\e[01;32m' \
        "${getExe pkgs.man-db}" "$@"
      }
    ''
    (mkAfter /* zsh */ ''
      vimd() {
        local arg="$1"
        local dir
        if [ -d "$arg" ]; then
          dir="$arg"
        else
          dir="$(dirname "$arg")"
        fi
        vim -c "cd $dir" "$arg"
      }
      unsymlink() {
        local target="$1"
        if [[ -L $target ]]; then
          cp --remove-destination "$(readlink -f "$target")" "$target"
          echo "Unsymlinked: $target"
        else
          echo "Not a symlink: $target"
        fi
      }
      nixos-update() {
        nix flake update --flake /etc/nixos
      }
      nixos-test() {
        sudo nixos-rebuild test "''${1:+--specialisation}" "''${1:+$1}" \
          --keep-going --log-format=internal-json -v \
          ${optionalString config.host.out-of-store-symlinks "--impure"} \
          |& nom --json
      }
      nixos-switch() {
        sudo nixos-rebuild switch "''${1:+--specialisation}" "''${1:+$1}" \
          --keep-going --log-format=internal-json -v \
          ${optionalString config.host.out-of-store-symlinks "--impure"} \
          |& nom --json
      }
      nixos-boot() {
        sudo nixos-rebuild boot \
          --keep-going --log-format=internal-json -v \
          ${optionalString config.host.out-of-store-symlinks "--impure"} \
          |& nom --json
      }
      ${
        if config.host.home-manager-nixos then
          /* zsh */ ''
            home-manager-switch() {
              echo "use nixos-switch, or nixos-boot"
              return 1
            }
          ''
        else
          # TODO: don't hardcode path
          /* zsh */ ''
            home-manager-switch() {
              home-manager switch --flake /etc/nixos \
                --keep-going --log-format internal-json -v \
                ${optionalString config.host.out-of-store-symlinks "--impure"} \
                |& nom --json
            }
          ''
      }
    '')
  ];
}
