# NOTE: this file is manually formatted
let
  # https://github.com/rockyzhang24/arctic.nvim/tree/v2
  arctic = {
    norm_fg = "#CCCCCC";
    norm_bg = "#1F1F1F";

    dark_red = "#D16969";
    orange = "#F9AE28";
    brown = "#CE9178";
    yellow = "#DCDCAA";
    yellow_orange = "#D7BA7D";
    green = "#6A9955";
    blue_green = "#4EC9B0";
    light_green = "#B5CEA8";
    blue = "#4FC1FF";
    light_blue = "#9CDCFE";
    dark_blue = "#569CD6";
    cornflower_blue = "#6796E6";
    dark_pink = "#C586C0";
    bright_pink = "#F92672";
    purple = "#AE81FF";

    white = "#FFFFFF";
    gray = "#51504F"; # StatuslineNC's fg
    gray2 = "#6E7681"; # LineNr (editorLineNumber.foreground)
    gray3 = "#808080";
    gray4 = "#9D9D9D";
    black = "#2D2D2D"; # TabLine
    black2 = "#252526";
    black3 = "#282828"; # CursorLine (editor.lineHighlightBorder). Or use #2a2d2e (list.hoverBackground) for a brighter color
    black4 = "#181818"; # Statusline

    error_red = "#F14C4C";
    warn_yellow = "#CCA700";
    info_blue = "#3794ff";
    hint_gray = "#B0B0B0";
    ok_green = "#89D185"; # color for success, so I use notebookStatusSuccessIcon.foreground

    selected_item_bg = "#04395E";
    matched_chars = "#2AAAFF";
    folded_blue = "#212D3A"; # editor.foldBackground
    float_border_fg = "#454545";
    indent_guide_fg = "#404040";
    indent_guide_scope_fg = "#707070";
    label_fg = "#C8C8C8";
    tab_border_fg = "#2B2B2B";
  };
  a = arctic;
in
{
  # builtin: https://neovim.io/doc/user/syntax.html#highlight-groups
  Normal = { fg = a.norm_fg; bg = a.norm_bg; };

  # default https://neovim.io/doc/user/syntax.html#group-name
  Comment = { fg = a.green; };

  Constant = { fg = a.blue; };
  String = { fg = a.brown; };
  Character.link = "String";
  Number = { fg = a.light_green; };
  Boolean = { fg = a.dark_blue; };
  Float.link = "Number";

  Identifier = { fg = a.light_blue; };
  Function = { fg = a.yellow; };

  Statement.link = "Normal";
  Conditional = { fg = a.dark_pink; };
  Repeat = { fg = a.dark_pink; };
  # TODO: i've never liked these having no color
  Label = { fg = a.norm_fg; }; 
  Operator = { fg = a.norm_fg; };

  Keyword = { fg = a.dark_blue; };
  Exception = { fg = a.dark_pink; };

  PreProc.link = "Macro"; 
  Include = { fg = a.dark_pink; };
  Define.link = "Macro";
  Macro = { fg = a.dark_blue; }; # TODO: macros should be different from kw
  PreCondit.link = "Conditional";

  Type = { fg = a.blue_green; };
  #StorageClass.link = "Keyword";
  #Structure.link = "Keyword";
  #Typedef.link = "Keyword";

  Special = { fg = a.norm_fg; underline = true; }; # TODO: improve
  SpecialChar = { fg = a.yellow_orange; };

  Todo = { bold = true; };

  # treesitter: https://neovim.io/doc/user/treesitter.html#treesitter-highlight-groups
  "@variable".link = "Identifier";
  #"@variable.builtin" = {};

  "@variable.parameter" = { italic = true; };

  "@constructor" = { fg = a.yellow_orange; };

  "@keyword.import".link = "Include";

  # lsp
  "@lsp.type.enumMember".link = "@constructor";

}
