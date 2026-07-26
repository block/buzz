# Zsh plugin for Buzz CLI completion
# Loads zsh completion script automatically if `buzz` is installed on PATH.

if (( $+commands[buzz] )); then
  source <(buzz completions zsh)
fi
