final: prev: {
  # docsets are otherwise only read from $XDG_DATA_HOME/fennel-ls/docsets, and macro modules
  # only from flsproject.fnl's macro-path; this adds $FENNEL_LS_DOCSETS and
  # $FENNEL_LS_MACRO_PATH, so a devshell can hand over files generated from its pins
  fennel-ls = prev.fennel-ls.overrideAttrs (old: {
    patches = old.patches or [ ] ++ [ ./fennel-ls-env-paths.patch ];
  });
}
