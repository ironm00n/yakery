{ forgejo-lts }:
forgejo-lts.overrideAttrs (old: {
  patches = old.patches ++ [ ./drop-unmatched.patch ];
})
