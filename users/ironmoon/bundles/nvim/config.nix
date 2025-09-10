{
  clipboard.providers.wl-copy.enable = true;
  colorschemes.catppuccin.enable = true;
  # colorschemes.vscode.enable = true;

  globals = {
    mapleader = " ";
  };
  opts = {
    number = true;
    expandtab = true;
    shiftwidth = 2;
    tabstop = 2;
  };

  plugins = {
    lualine.enable = true;
    which-key.enable = true;
    whitespace.enable = true;
    telescope = {
      enable = true;
      keymaps = {
        "<leader>fg" = "live_grep";
        "<leader>ff" = "find_files";
        "<leader>fb" = "buffers";
        "<leader>fm" = "man_pages";
      };
    };
    treesitter = {
      enable = true;
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
      autoEnableSources = true;
      settings.sources = [
        { name = "nvim_lsp"; }
        { name = "path"; }
        { name = "buffer"; }
      ];
    };
    lspconfig = {
      enable = true;
    };
    direnv.enable = true;
  };

  keymaps = [
    {
      mode = "n";
      key = "<leader>gg";
      action = "<cmd>LazyGit<CR>";
      options.desc = "lazygit";
    }
  ];

  lsp = {
    servers = {
      merlin.enable = true;
      ocamllsp.enable = true;
      ocamllsp.package = null;
    };
  };
}
