---@param mod string
function opt_require(mod)
  local status, value = pcall(require, mod)
  if not status then
    print("failed to load module, its error message was:", value)
  end
end
