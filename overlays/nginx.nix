final: prev: {
  # nginx 1.30.7 fixes CVE-2026-123007 and CVE-2026-123633 (HTTP/2) among others; nixpkgs master has it, the pinned
  # nixpkgs not yet. Master's package differs from this one only in version and hash.
  nginxStable =
    prev.lib.warnIf (prev.lib.versionAtLeast prev.nginxStable.version "1.30.7")
      "nixpkgs has nginx 1.30.7; drop overlays/nginx.nix"
      (
        prev.nginxStable.overrideAttrs rec {
          version = "1.30.7";
          nginxVersion = version;
          src = prev.fetchurl {
            url = "https://nginx.org/download/nginx-${version}.tar.gz";
            hash = "sha256-JWW1yDcFDpWPcPsPc8SOK8pFQq00n9NW9G+wxyYQvG4=";
          };
        }
      );
}
