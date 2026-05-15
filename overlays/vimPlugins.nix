final: prev:
let
  inherit (final.vimUtils.override { inherit (final) vim; }) buildVimPlugin;
in
{
  # 0.4.2 was marked unfree, 0.5.0 fixes this (added MIT licence)
  vimPlugins = prev.vimPlugins.extend (
    vfinal: vprev: {
      whitespace-nvim = buildVimPlugin {
        pname = "whitespace.nvim";
        version = "0.5.0";
        src = prev.fetchFromGitHub {
          owner = "johnfrankmorgan";
          repo = "whitespace.nvim";
          tag = "0.5.0";
          hash = "sha256-d+jbLU5N4qJ4WzPPHWZWPTesZ++h6TiYc5z5sYlbDgE=";
        };
        meta.homepage = "https://github.com/johnfrankmorgan/whitespace.nvim/";
      };
    }
  );
}
