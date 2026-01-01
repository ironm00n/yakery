# TODO:
# - spellcheck
{ pkgs, ... }:
let
  my-hl = false;

  inherit (pkgs.tree-sitter) buildGrammar;
  inherit (pkgs) fetchFromGitHub;

  # doesn't package queries
  tree-sitter-pyret = buildGrammar {
    language = "pyret";
    version = "0.0.0";
    src = fetchFromGitHub {
      owner = "ironm00n";
      repo = "tree-sitter-pyret";
      rev = "faeac9ce224b63e363de46c8ba816a4b4f930993";
      hash = "sha256-BZTTh6DGTiZk8fTcl2wz1YLzFm/WLJntrRxEMX5dEMQ=";
    };
  };

  # doesn't package queries
  tree-sitter-wasm = buildGrammar {
    language = "wat";
    version = "0.0.0+rev=2ca28a9";
    src = fetchFromGitHub {
      owner = "wasm-lsp";
      repo = "tree-sitter-wasm";
      rev = "2ca28a9f9d709847bf7a3de0942a84e912f59088";
      hash = "sha256-a1l4RsGpRQfUxEjwewyKiV0G7J2DHZW6+y1HnjREYAs=";
    };
    location = "wat";
  };

  tree-sitter-kitty = buildGrammar {
    language = "kitty";
    version = "0.0.0+rev=2e9b602";
    src = fetchFromGitHub {
      owner = "OXY2DEV";
      repo = "tree-sitter-kitty";
      rev = "2e9b602ca676cac63887cca5a4535106f3475c82";
      hash = "sha256-9knYf4/0G8zX2grWJi6U/1TQmUWQCjdMK3Vd/fw93C0=";
    };
    patchPhase = ''
      mkdir -p queries/kitty
      mv queries/*.scm queries/kitty/
    '';
  };

  tree-sitter-zsh = buildGrammar {
    language = "zsh";
    version = "0.42.0";
    src = fetchFromGitHub {
      owner = "georgeharker";
      repo = "tree-sitter-zsh";
      rev = "v0.42.0";
      hash = "sha256-atPMgFt23gmhKorBnMuwmn2eLpWLfE/7dyD05CBg2cc=";
    };
    patchPhase = ''
      mkdir -p queries/zsh
      rm queries/*.scm
      mv nvim-queries/*.scm queries/zsh/
    '';
  };

  tree-sitter-plugins = [
    tree-sitter-pyret
    tree-sitter-wasm
    tree-sitter-kitty
    tree-sitter-zsh
  ];
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
      grammarPackages = pkgs.vimPlugins.nvim-treesitter.passthru.allGrammars ++ tree-sitter-plugins;
      settings = {
        folding = true;
        highlight = {
          enable = true;
        };
      };
      luaConfig.post = ''
        do
          local parser_config = require("nvim-treesitter.parsers").get_parser_configs()
          parser_config.pyret = {
            install_info = {
              url = "${tree-sitter-pyret}",
              files = {"src/parser.c", "src/scanner.c"},
            }
          }
          parser_config.kitty = {
            install_info = {
              url = "${tree-sitter-kitty}",
              file = {"src/parser.c"},
            }
          }

          vim.api.nvim_create_autocmd({ "BufRead", "BufNewFile" }, {
            pattern = "*.conf",
            callback = function (event)
              local path = event.match;
              if string.match(path, "kitty%.conf$") then
                vim.bo[event.buf].ft = "kitty";
              end
            end
          });
        end
      '';
    };
    ts-context-commentstring = {
      enable = true;
      autoLoad = true;
      settings = {
        enable_autocmd = false;
        languages = {
          kitty = "# %s";
          pyret = {
            __default = "# %s";
            __multiline = "#| %s |#";
          };
        };
      };
    };
    comment = {
      enable = true;
      settings.pre_hook = "require('ts_context_commentstring.integrations.comment_nvim').create_pre_hook()";
    };
    treesitter-refactor = {
      enable = true;
      settings = {
        smart_rename = {
          enable = true;
          keymaps.smart_rename = "gtr";
        };
      };
    };

    neo-tree = {
      enable = true;
      settings = {
        filesystem.filtered_items = {
          visible = true;
          hide_dot_files = false;
          hide_by_name = [ ".git" ];
        };
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
    nvim-autopairs.enable = true;
    gitsigns.enable = true;
    toggleterm.enable = true;
  };

  extraPlugins = tree-sitter-plugins;

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
      rust_analyzer.enable = true;
      rust_analyzer.package = null;
    };
  };

  extraConfigLua = ''
    -- Remove the 'How-to disable mouse' popup entry and its separator
    vim.cmd([[aunmenu PopUp.How-to\ disable\ mouse]])
    vim.cmd([[aunmenu PopUp.-2-]])

    -- tree-sitter does a better job
    vim.api.nvim_set_hl(0, '@lsp.type.string.ocaml', {})
    vim.api.nvim_set_hl(0, '@lsp.type.comment.nix', {})

    function set_terminal_keymaps()
      local opts = { buffer = 0 }
      vim.keymap.set('t', '<esc>', [[<C-\><C-n>]], opts)
      vim.keymap.set('t', 'jk',    [[<C-\><C-n>]], opts)
      vim.keymap.set('t', '<C-h>', [[<Cmd>wincmd h<CR>]], opts)
      vim.keymap.set('t', '<C-j>', [[<Cmd>wincmd j<CR>]], opts)
      vim.keymap.set('t', '<C-k>', [[<Cmd>wincmd k<CR>]], opts)
      vim.keymap.set('t', '<C-l>', [[<Cmd>wincmd l<CR>]], opts)
      vim.keymap.set('t', '<C-w>', [[<C-\><C-n><C-w>]], opts)
    end

    vim.api.nvim_create_autocmd("TermOpen", {
      pattern = "term://*toggleterm#*",
      callback = set_terminal_keymaps,
    })
  '';
}
