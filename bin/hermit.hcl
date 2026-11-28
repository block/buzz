sources = ["env:///bin/packages", "https://github.com/cashapp/hermit-packages.git"]
manage-git = true

env = {
  "CARGO_HOME": "${HOME}/.cargo",
}
