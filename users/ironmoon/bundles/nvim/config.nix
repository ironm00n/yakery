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
        "<leader>fm" = "man_pages"; # TODO: this is broken
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
  ];

  lsp = {
    inlayHints.enable = true;
    servers = {
      merlin.enable = true;
      ocamllsp.enable = true;
      ocamllsp.package = null;
      nil_ls.enable = true;
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
