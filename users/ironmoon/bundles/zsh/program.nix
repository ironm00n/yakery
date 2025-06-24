{
  lib,
  config,
  pkgs,
}:
let
  inherit (lib)
    mkMerge
    mkBefore
    mkAfter
    getExe
    ;
in
{
  enable = true;
  defaultKeymap = "emacs";

  # note that i am using antidote over built=in homemanager for highlighting etc
  enableCompletion = true;
  completionInit = ''
    autoload -zU compinit
    compinit -C -u -d "$ZSH_CACHE_DIR/zcompdump"
  '';
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
    size = 100000;
    path = "${config.xdg.dataHome}/zsh/history";
  };
  antidote = {
    enable = true;
    plugins = [
      "romkatv/powerlevel10k"
      "zsh-users/zsh-syntax-highlighting"
      "zsh-users/zsh-autosuggestions"
      "zsh-users/zsh-history-substring-search"
    ];
  };
  # This ensures that the Powerlevel10k instant prompt is enabled
  # it also defines the $IS_TTY variable, which is used to determine if we are in a TTY
  # so that we don't try rendering weird characters in a basic TTY terminal
  initContent = mkMerge [
    (mkBefore /* zsh */ ''
      case $(tty) in
        (/dev/tty[1-9]) IS_TTY=1;;
                    (*) IS_TTY=0;;
      esac

      if ! (($IS_TTY)); then
        # see: https://github.com/romkatv/powerlevel10k/issues/702#issuecomment-626222730
        emulate zsh -c "$(${getExe pkgs.direnv} export zsh)"

        if [[ -r "''${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-''${(%):-%n}.zsh" ]]; then
          source "''${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-''${(%):-%n}.zsh"
        fi

        emulate zsh -c "$(${getExe pkgs.direnv} hook zsh)"
      else
        eval "$(${getExe pkgs.direnv} hook zsh)"
      fi

      ZSH_CACHE_DIR="''${XDG_CACHE_HOME:-$HOME/.cache}/zsh"
      mkdir -p "$ZSH_CACHE_DIR"
    '')
    /* zsh */
    ''
      if ! (($IS_TTY)); then
        [[ -f ~/.p10k.zsh ]] && source ~/.p10k.zsh
      fi

      bindkey '^[[3~' delete-char         # del
      bindkey '^H' backward-kill-word     # ctrl + backspace
      bindkey '^[[3;5~' kill-word         # ctrl + del
      bindkey '^[[1;5D' backward-word     # ctrl + left
      bindkey '^[[1;5C' forward-word      # ctrl + right

      bindkey '^[[Z' reverse-menu-complete

      bindkey '^[[A' history-substring-search-up
      bindkey '^[OA' history-substring-search-up
      bindkey '^[[B' history-substring-search-down
      bindkey '^[OB' history-substring-search-down

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

      export GROFF_NO_SGR=1
      export LESS_TERMCAP_ue=$'\e[00m'
      export LESS_TERMCAP_se=$'\e[00m'
      export LESS_TERMCAP_mb=$'\e[01;31m'
      export LESS_TERMCAP_md=$'\e[01;31m'
      export LESS_TERMCAP_me=$'\e[00m'
      export LESS_TERMCAP_so=$'\e[01;33m\e[44m'
      export LESS_TERMCAP_us=$'\e[01;32m'
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
      # TODO: use current specialisation
      nixos-switch() {
        sudo nixos-rebuild switch "''${1:+--specialisation}" "''${1:+$1}" \
          --keep-going --log-format=internal-json -v |& nom --json
      }
      nixos-boot() {
        sudo nixos-rebuild boot \
          --keep-going --log-format=internal-json -v |& nom --json
      }
    '')
  ];
}
