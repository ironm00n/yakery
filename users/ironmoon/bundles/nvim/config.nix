# TODO:
# - spellcheck
{ pkgs, ... }:
let
  my-hl = false;

  tree-sitter-pyret = pkgs.tree-sitter.buildGrammar {
    language = "pyret";
    version = "0.0.0+";
    src = pkgs.fetchFromGitHub {
      owner = "ironm00n";
      repo = "tree-sitter-pyret";
      rev = "faeac9ce224b63e363de46c8ba816a4b4f930993";
      hash = "sha256-BZTTh6DGTiZk8fTcl2wz1YLzFm/WLJntrRxEMX5dEMQ=";
    };
  };

  tree-sitter-wasm = pkgs.tree-sitter.buildGrammar {
    language = "wat";
    version = "0.0.0+rev=2ca28a9";
    src = pkgs.fetchFromGitHub {
      owner = "wasm-lsp";
      repo = "tree-sitter-wasm";
      rev = "2ca28a9f9d709847bf7a3de0942a84e912f59088";
      hash = "sha256-a1l4RsGpRQfUxEjwewyKiV0G7J2DHZW6+y1HnjREYAs=";
    };
    location = "wat";
  };
in
{
  highlight = if my-hl then import ./highlight.nix else { };

  clipboard.providers.wl-copy.enable = true;
  colorschemes.catppuccin = {
    enable = !my-hl;
    settings = {
      no_bold = true;
      styles = {
        comments = [ ];
        conditionals = [ ];
      };
    };
  };

  globals = {
    mapleader = " ";
  };
  opts = {
    number = true;
    expandtab = true;
    shiftwidth = 2;
    tabstop = 2;
  };

  extraFiles = {
    "queries/pyret/fold.scm".source = ./queries/pyret/folds.scm;
    "queries/pyret/highlights.scm".source = ./queries/pyret/highlights.scm;
  };

  plugins = {
    lualine.enable = true;
    bufferline.enable = true;
    which-key.enable = true;
    whitespace.enable = true;
    telescope = {
      enable = true;
      keymaps = {
        "<leader>fg" = "live_grep";
        "<leader>ff" = "find_files";
        "<leader>fb" = "buffers";
        "<leader>fm" = "man_pages"; # TODO: this is broken
      };
    };
    treesitter = {
      enable = true;
        grammarPackages = pkgs.vimPlugins.nvim-treesitter.passthru.allGrammars ++ [
          tree-sitter-pyret
          tree-sitter-wasm
        ];
      settings = {
        folding = true;
        highlight = {
          enable = true;
        };
      };
    };
    neo-tree = {
      enable = true;
      filesystem.filteredItems = {
        visible = true;
        hideDotfiles = false;
        hideByName = [
          ".git"
        ];
      };
    };
    web-devicons = {
      enable = true;
    };
    alpha = {
      enable = false;
      theme = "dashboard";
    };
    lazygit = {
      enable = true;
    };
    cmp = {
      enable = true;
      autoEnableSources = true;
      settings = {
        sources = [
          { name = "nvim_lsp"; }
          { name = "path"; }
          { name = "buffer"; }
        ];
        mapping = {
          "<C-d>" = "cmp.mapping.scroll_docs(-4)";
          "<C-f>" = "cmp.mapping.scroll_docs(4)";
          "<C-Space>" = "cmp.mapping.complete()";
          "<C-e>" = "cmp.mapping.close()";
          "<Tab>" = "cmp.mapping(cmp.mapping.select_next_item(), {'i', 's'})";
          "<S-Tab>" = "cmp.mapping(cmp.mapping.select_prev_item(), {'i', 's'})";
          "<CR>" = "cmp.mapping.confirm({ select = true })"; # TODO: i don't think i like this
        };
      };
    };
    lspconfig.enable = true;
    direnv.enable = true;
    comment.enable = true;
    nvim-autopairs.enable = true;
    gitsigns.enable = true;
    toggleterm.enable = true;
  };

  keymaps = [
    {
      mode = "n";
      key = "<leader>gg";
      action = "<cmd>LazyGit<CR>";
      options.desc = "lazygit";
    }
    {
      mode = "n";
      key = "<C-n>";
      action = "<cmd>Neotree toggle<CR>";
    }
    {
      mode = "n";
      key = "<leader>d";
      action.__raw = "function() vim.diagnostic.open_float() end";
      options.silent = true;
    }
    {
      mode = "n";
      key = "<leader>t";
      action = "<cmd>ToggleTerm direction=float<CR>";
    }
    {
      mode = "t";
      key = "<esc>";
      action = "<C-\\><C-n>";
    }
    {
      mode = "n";
      key = "<leader>x";
      action = "<cmd>x<CR>";
    }
    {
      mode = "n";
      key = "<leader>/";
      action.__raw = ''
        function() require("Comment.api").toggle.linewise.current() end
      '';
    }
  ];

  lsp = {
    inlayHints.enable = true;
    servers = {
      merlin.enable = true;
      ocamllsp.enable = true;
      ocamllsp.package = null;
      nil_ls.enable = true;
      wasm_language_tools.enable = true;
    };
  };

  extraConfigLua = ''
    -- Remove the 'How-to disable mouse' popup entry and its separator
    vim.cmd([[aunmenu PopUp.How-to\ disable\ mouse]])
    vim.cmd([[aunmenu PopUp.-2-]])

    -- tree-sitter does a better job
    vim.api.nvim_set_hl(0, '@lsp.type.string.ocaml', {})
    vim.api.nvim_set_hl(0, '@lsp.type.comment.nix', {})
  '';
}
