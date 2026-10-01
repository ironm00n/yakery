# NixOS builds its font cache with none of conf.d (fonts.conf includes it as
# /etc/fonts/conf.d, absent in the sandbox), so target="scan" rules never reach
# system fonts. This rebuilds the cache under the generated conf.d and swaps it in.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkEnableOption mkForce mkIf;
  cfg = config.bundles.fontconfig-scan-cache;
  fc = config.fonts.fontconfig;
  cacheConfName = "00-nixos-cache.conf";

  fcConf = body: ''
    <?xml version="1.0"?>
    <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
    <fontconfig>
    ${body}
    </fontconfig>
  '';

  fontDirs = pkgs.writeText "fc-font-dirs.conf" (
    fcConf (lib.concatMapStrings (d: "<dir>${d}</dir>\n") config.fonts.packages)
  );

  confEtc = pkgs.buildEnv {
    name = "fontconfig-etc";
    paths = fc.confPackages;
    ignoreCollisions = true;
  };

  mkCache =
    fontconfig:
    pkgs.runCommand "fc-cache"
      {
        preferLocalBuild = true;
        allowSubstitutes = false;
      }
      ''
        export FONTCONFIG_FILE=$PWD/fonts.conf FONTCONFIG_PATH=${confEtc}/etc/fonts
        {
          echo '<?xml version="1.0"?><fontconfig>'
          echo '<include>${fontconfig.out}/etc/fonts/fonts.conf</include>'
          for f in ${confEtc}/etc/fonts/conf.d/*.conf; do
            if [ "''${f##*/}" = ${cacheConfName} ]; then
              replaced=1
            else
              echo "<include>$f</include>"
            fi
          done
          echo '<include>${fontDirs}</include>'
          echo "<cachedir>$out</cachedir></fontconfig>"
        } > fonts.conf
        if [ -z "$replaced" ]; then
          echo "fonts.fontconfig.confPackages no longer provide ${cacheConfName}" >&2
          exit 1
        fi
        mkdir -p $out
        ${lib.getExe' fontconfig "fc-cache"} -s
        rm -f $out/CACHEDIR.TAG
      '';

  cacheConf = pkgs.writeTextDir "etc/fonts/conf.d/${cacheConfName}" (fcConf ''
    <include>${fontDirs}</include>
    <cachedir>${mkCache pkgs.fontconfig}</cachedir>
    ${lib.optionalString fc.cache32Bit "<cachedir>${mkCache pkgs.pkgsi686Linux.fontconfig}</cachedir>"}
  '');
in
{
  options.bundles.fontconfig-scan-cache = {
    enable = mkEnableOption "a font cache built under the system's fontconfig rules";
  };

  config = mkIf (cfg.enable && fc.enable) {
    environment.etc.fonts.source = mkForce "${
      pkgs.buildEnv {
        name = "fontconfig-etc";
        paths = [ (lib.hiPrio cacheConf) ] ++ fc.confPackages;
        ignoreCollisions = true;
      }
    }/etc/fonts/";
  };
}
