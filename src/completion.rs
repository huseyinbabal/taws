//! Custom shell completion scripts with dynamic profile/region completion
//!
//! These scripts extend the basic clap-generated completions to add
//! dynamic completion for --profile and --region arguments by calling
//! `taws list-profiles` and `taws list-regions`.

/// Generate bash completion script with dynamic profile/region completion
pub fn generate_bash() -> String {
    r#"_taws() {
    local i cur prev opts cmd
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    cmd=""
    opts=""

    # Handle --profile completion
    if [[ ${prev} == "-p" || ${prev} == "--profile" ]]; then
        local profiles
        profiles=$(taws list-profiles 2>/dev/null)
        COMPREPLY=( $(compgen -W "${profiles}" -- "${cur}") )
        return 0
    fi

    # Handle --region completion
    if [[ ${prev} == "-r" || ${prev} == "--region" ]]; then
        local regions
        regions=$(taws list-regions 2>/dev/null)
        COMPREPLY=( $(compgen -W "${regions}" -- "${cur}") )
        return 0
    fi

    # Handle --log-level completion
    if [[ ${prev} == "--log-level" ]]; then
        COMPREPLY=( $(compgen -W "off error warn info debug trace" -- "${cur}") )
        return 0
    fi

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"; do
        case "${cmd},${i}" in
            ",$1")
                cmd="taws"
                ;;
            taws,completion)
                cmd="taws__completion"
                ;;
            taws,help)
                cmd="taws__help"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        taws)
            opts="-p -r -h -V --profile --region --log-level --readonly --endpoint-url --help --version completion help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]]; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            ;;
        taws__completion)
            opts="-h --help bash zsh fish powershell elvish"
            if [[ ${cur} == -* ]]; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            COMPREPLY=( $(compgen -W "bash zsh fish powershell elvish" -- "${cur}") )
            return 0
            ;;
        taws__help)
            opts="completion help"
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
    complete -F _taws -o nosort -o bashdefault -o default taws
else
    complete -F _taws -o bashdefault -o default taws
fi
"#
    .to_string()
}

/// Generate zsh completion script with dynamic profile/region completion
pub fn generate_zsh() -> String {
    r##"#compdef taws

autoload -U is-at-least

_taws_profiles() {
    local profiles
    profiles=(${(f)"$(taws list-profiles 2>/dev/null)"})
    _describe -t profiles 'AWS profiles' profiles
}

_taws_regions() {
    local regions
    regions=(${(f)"$(taws list-regions 2>/dev/null)"})
    _describe -t regions 'AWS regions' regions
}

_taws() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
        '-p+[AWS profile to use]:PROFILE:_taws_profiles' \
        '--profile=[AWS profile to use]:PROFILE:_taws_profiles' \
        '-r+[AWS region to use]:REGION:_taws_regions' \
        '--region=[AWS region to use]:REGION:_taws_regions' \
        '--log-level=[Log level for debugging]:LOG_LEVEL:(off error warn info debug trace)' \
        '--endpoint-url=[Custom AWS endpoint URL]:ENDPOINT_URL:_default' \
        '--readonly[Run in read-only mode]' \
        '-h[Print help]' \
        '--help[Print help]' \
        '-V[Print version]' \
        '--version[Print version]' \
        ":: :_taws_commands" \
        "*::: :->taws" \
        && ret=0

    case $state in
    (taws)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:taws-command-$line[1]:"
        case $line[1] in
            (completion)
                _arguments "${_arguments_options[@]}" : \
                    '-h[Print help]' \
                    '--help[Print help]' \
                    ':shell:(bash zsh fish powershell elvish)' \
                    && ret=0
                ;;
            (help)
                _arguments "${_arguments_options[@]}" : \
                    ":: :_taws_help_commands" \
                    "*::: :->help" \
                    && ret=0
                ;;
        esac
        ;;
    esac

    return ret
}

_taws_commands() {
    local commands; commands=(
        'completion:Generate shell completion scripts'
        'help:Print help for the given subcommand(s)'
    )
    _describe -t commands 'taws commands' commands "$@"
}

_taws_help_commands() {
    local commands; commands=(
        'completion:Generate shell completion scripts'
        'help:Print help for the given subcommand(s)'
    )
    _describe -t commands 'taws help commands' commands "$@"
}

if [ "$funcstack[1]" = "_taws" ]; then
    _taws "$@"
else
    compdef _taws taws
fi
"##
    .to_string()
}

/// Generate fish completion script with dynamic profile/region completion
pub fn generate_fish() -> String {
    r#"# Fish completion for taws

# Disable file completion by default
complete -c taws -f

# Dynamic profile completion
complete -c taws -n "__fish_seen_subcommand_from -p --profile" -xa "(taws list-profiles 2>/dev/null)"
complete -c taws -s p -l profile -d 'AWS profile to use' -xa "(taws list-profiles 2>/dev/null)"

# Dynamic region completion  
complete -c taws -n "__fish_seen_subcommand_from -r --region" -xa "(taws list-regions 2>/dev/null)"
complete -c taws -s r -l region -d 'AWS region to use' -xa "(taws list-regions 2>/dev/null)"

# Log level completion
complete -c taws -l log-level -d 'Log level for debugging' -xa "off error warn info debug trace"

# Other options
complete -c taws -l readonly -d 'Run in read-only mode'
complete -c taws -l endpoint-url -d 'Custom AWS endpoint URL'
complete -c taws -s h -l help -d 'Print help'
complete -c taws -s V -l version -d 'Print version'

# Subcommands
complete -c taws -n "__fish_use_subcommand" -a "completion" -d 'Generate shell completion scripts'
complete -c taws -n "__fish_use_subcommand" -a "help" -d 'Print help for subcommand(s)'

# Completion subcommand
complete -c taws -n "__fish_seen_subcommand_from completion" -xa "bash zsh fish powershell elvish"
"#
    .to_string()
}

/// Generate PowerShell completion script with dynamic profile/region completion
pub fn generate_powershell() -> String {
    r#"using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'taws' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'taws'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
            }
            $element.Value
        }
    ) -join ';'

    $completions = @()
    
    # Check if we're completing --profile or -p value
    $lastArg = $commandElements[-2].Value
    if ($lastArg -eq '--profile' -or $lastArg -eq '-p') {
        $profiles = taws list-profiles 2>$null
        if ($profiles) {
            $profiles | ForEach-Object {
                if ($_ -like "$wordToComplete*") {
                    $completions += [CompletionResult]::new($_, $_, 'ParameterValue', $_)
                }
            }
        }
        return $completions
    }
    
    # Check if we're completing --region or -r value
    if ($lastArg -eq '--region' -or $lastArg -eq '-r') {
        $regions = taws list-regions 2>$null
        if ($regions) {
            $regions | ForEach-Object {
                if ($_ -like "$wordToComplete*") {
                    $completions += [CompletionResult]::new($_, $_, 'ParameterValue', $_)
                }
            }
        }
        return $completions
    }
    
    # Check if we're completing --log-level value
    if ($lastArg -eq '--log-level') {
        @('off', 'error', 'warn', 'info', 'debug', 'trace') | ForEach-Object {
            if ($_ -like "$wordToComplete*") {
                $completions += [CompletionResult]::new($_, $_, 'ParameterValue', $_)
            }
        }
        return $completions
    }

    switch ($command) {
        'taws' {
            @('--profile', '-p', '--region', '-r', '--log-level', '--readonly', '--endpoint-url', '--help', '-h', '--version', '-V', 'completion', 'help') | ForEach-Object {
                if ($_ -like "$wordToComplete*") {
                    $completions += [CompletionResult]::new($_, $_, 'ParameterName', $_)
                }
            }
        }
        'taws;completion' {
            @('bash', 'zsh', 'fish', 'powershell', 'elvish', '--help', '-h') | ForEach-Object {
                if ($_ -like "$wordToComplete*") {
                    $completions += [CompletionResult]::new($_, $_, 'ParameterValue', $_)
                }
            }
        }
        'taws;help' {
            @('completion', 'help') | ForEach-Object {
                if ($_ -like "$wordToComplete*") {
                    $completions += [CompletionResult]::new($_, $_, 'ParameterValue', $_)
                }
            }
        }
    }

    return $completions
}
"#
    .to_string()
}
