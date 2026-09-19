#!/usr/bin/env bash

# ==============================================================================
# Competitive Programming Runner
# ==============================================================================
#
# Usage:
#   ./rn.sh [--debug] <source> [command] [arguments]
#   ./rn.sh [--debug] <command> <source> [arguments]
#   ./rn.sh --source <source> <command> [arguments]
#
#   Source extensions: .cpp, .py, .c, .java, .rs, .go, .kt. No extension
#   means .cpp. The command and source can swap positions; use --source when
#   file operands make the source ambiguous. --debug can appear anywhere.
#   Short options require a dash (for example, -p rather than p).
#
#   Run and build:
#     (none)              Run interactively (compile when needed)
#     -c                  Compile only; -d compiles with debug flags
#                         Both are unsupported for Python.
#     --timeout <seconds> Run interactively with a time limit
#
#   Input and output:
#     -s, --save          Run interactively; save input as <stem>.in<ID>
#     -i                  Run with <stem>.in1
#     -p                  Clipboard -> <stem>.in1 -> run
#     -pa                 Clipboard -> next <stem>.in<ID> -> run
#     -f, --in <files>    Run once per input file
#     -o, --out <files>   Save stdout; pair with -f/--in when both are used
#
#   Saved cases:
#     --all               Run every saved case; --id <IDs> runs selected IDs
#     --last              Run the most recently modified saved case
#     --list              Show saved IDs, metadata, and contents
#     --show <ID>         Show one saved case
#     --copy <ID>         Copy a saved case to the clipboard
#     --delete <ID>       Delete one saved case; --clear deletes all cases
#
#   Compare and stress:
#     --diff <ID> <file>  Compare a saved case with expected output
#     --stress <brute> <generator>  Test generated cases against a brute
#
#   ./rn.sh --help       Show detailed syntax and examples
#
# Environment:
#   CXX=clang++ ./rn.sh A        Use a different C++ compiler
#   KOTLINC=/path/to/kotlinc ./rn.sh A.kt  Use a different Kotlin compiler
#   TIME_BIN=gtime ./rn.sh A -i  Override the GNU time executable
#   NO_COLOR=1 ./rn.sh A -i      Disable terminal colors
#
# Requirements:
#   A compiler or runtime for the chosen language, /usr/bin/time, tee, mkfifo,
#   timeout, realpath, sha256sum, flock; clipboard modes need wl-copy, wl-paste, or xclip
# ==============================================================================

CXX="${CXX:-g++}"
CC="${CC:-gcc}"
PYTHON="${PYTHON:-python3}"
JAVAC="${JAVAC:-javac}"
JAVA="${JAVA:-java}"
RUSTC="${RUSTC:-rustc}"
GO="${GO:-go}"
KOTLINC="${KOTLINC:-kotlinc}"
TIME_BIN="${TIME_BIN:-/usr/bin/time}"

STANDARD_FLAGS=(-std=c++17 -Wall -Wshadow -Wno-unused-result -DLOCAL)
DEBUG_FLAGS=(
    -std=c++17
    -DLOCAL
    -g3
    -Og
    -Wall
    -Wextra
    -pedantic
    -Wshadow
    -Wformat=2
    -Wfloat-equal
    -Wconversion
    -Wlogical-op
    -Wshift-overflow=2
    -Wduplicated-cond
    -Wduplicated-branches
    -Wcast-qual
    -Wcast-align
    -Wno-unused-result
    -D_GLIBCXX_DEBUG
    -D_GLIBCXX_DEBUG_PEDANTIC
    -D_FORTIFY_SOURCE=2
    -fsanitize=address,undefined
    -fno-sanitize-recover=all
    -fno-omit-frame-pointer
    -fstack-protector-strong
)

C_FLAGS=(-std=c17 -Wall -Wextra -Wshadow -Wno-unused-result)
C_DEBUG_FLAGS=(
    -std=c17 -g3 -Og -Wall -Wextra -pedantic -Wshadow -Wconversion
    -Wformat=2 -Wno-unused-result -D_FORTIFY_SOURCE=2
    -fsanitize=address,undefined -fno-sanitize-recover=all
    -fno-omit-frame-pointer -fstack-protector-strong
)


# ── Terminal style ────────────────────────────────────────────────────────────

grey=''
silver=''
red=''
green=''
cyan=''
orange=''
bold=''
dim=''
reset=''

if [[ -z ${NO_COLOR:-} ]] && [[ -t 1 || -t 2 ]] && command -v tput >/dev/null; then
    grey=$(tput setaf 8 2>/dev/null || true)
    silver=$(tput setaf 7 2>/dev/null || true)
    red=$(tput setaf 1 2>/dev/null || true)
    green=$(tput setaf 2 2>/dev/null || true)
    cyan=$(tput setaf 51 2>/dev/null || true)
    orange=$(tput setaf 208 2>/dev/null || true)
    bold=$(tput bold 2>/dev/null || true)
    dim=$(tput dim 2>/dev/null || true)
    reset=$(tput sgr0 2>/dev/null || true)
fi


# Return a sensible UI width, capped so wide terminals stay readable.
terminal_width() {
    local width=${COLUMNS:-}

    if [[ ! $width =~ ^[0-9]+$ ]]; then
        width=$(tput cols 2>/dev/null || printf '72')
    fi

    (( width < 48 )) && width=48
    (( width > 88 )) && width=88

    printf '%d' "$width"
}


repeat_char() {
    local character=$1
    local count=$2
    local padding

    (( count > 0 )) || return 0
    printf -v padding '%*s' "$count" ''
    printf '%s' "${padding// /$character}"
}


# Bash printf measures multibyte strings inconsistently across implementations
# when applying a field width. Pad explicitly so symbols such as ✓ and · do not
# pull the right table border out of alignment.
pad_right() {
    local text=$1
    local width=$2
    local padding=$(( width - ${#text} ))

    printf '%s' "$text"
    repeat_char ' ' "$padding"
}


# Print a full-width section banner such as:
#   ━━ INPUT · A.in1 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
ui_header() {
    local title=$1
    local detail=${2:-}
    local color=${3:-$cyan}
    local emphasis=$bold
    local width label fill

    (( $# >= 4 )) && emphasis=$4

    width=$(terminal_width)
    label="━━ $title"
    [[ -n $detail ]] && label+=" · $detail"
    label+=' '

    fill=$(( width - ${#label} ))
    (( fill < 2 )) && fill=2

    # Keep the rule in the same color/style as the section label.
    printf '%b%b%s%b%b%b' \
        "$color" "$emphasis" "$label" "$reset" "$color" "$emphasis"
    repeat_char '━' "$fill"
    printf '%b\n' "$reset"
}


ui_footer() {
    local style=$grey
    (( $# >= 1 )) && style=$1

    printf '%b' "$style"
    repeat_char '━' "$(terminal_width)"
    printf '%b\n' "$reset"
}


# Print the top of a bordered panel with its title embedded in the border.
box_top() {
    local title=$1
    local detail=${2:-}
    local color=${3:-$orange}
    local border_style=$grey
    local width label max_label fill

    (( $# >= 4 )) && border_style=$4

    width=$(terminal_width)
    label=" $title"
    [[ -n $detail ]] && label+=" · $detail"
    label+=' '

    # Keep very long filenames from pushing the right border off-screen.
    max_label=$(( width - 5 ))
    if (( ${#label} > max_label )); then
        label="${label:0:max_label-2}… "
    fi

    fill=$(( width - ${#label} - 3 ))

    printf '%b┌─%b%b%s%b%b' \
        "$border_style" "$reset" "$color" "$label" "$reset" "$border_style"
    repeat_char '─' "$fill"
    printf '┐%b\n' "$reset"
}


error() {
    printf '%b%bError:%b %s\n' "$red" "$bold" "$reset" "$1" >&2
}


# Print an input file unchanged. Add a display-only newline when its final line
# has no newline, so the next UI section cannot join that line.
show_file() {
    local input_file=$1
    local final_byte

    cat -- "$input_file" || return $?
    if [[ -s $input_file ]]; then
        final_byte=$(tail -c 1 -- "$input_file") || return $?
        [[ -n $final_byte ]] && printf '\n'
    fi
    return 0
}


# Copy the current desktop clipboard into a file. Prefer wl-paste in a Wayland
# session and fall back to xclip for X11.
paste_clipboard() {
    local destination=$1

    if command -v wl-paste >/dev/null 2>&1; then
        wl-paste > "$destination"
    elif command -v xclip >/dev/null 2>&1; then
        xclip -selection clipboard -o > "$destination"
    else
        error "Clipboard mode requires 'wl-paste' or 'xclip'."
        return 1
    fi
}


# Keep an existing input intact if clipboard access fails partway through.
paste_clipboard_atomic() {
    local destination=$1
    local temporary

    temporary=$(mktemp "$destination.tmp.XXXXXX") || {
        error "Could not create a temporary input beside '$destination'."
        return 1
    }

    if ! paste_clipboard "$temporary"; then
        rm -f -- "$temporary"
        error 'Could not read the clipboard.'
        return 1
    fi

    if ! mv -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        error "Could not save '$destination'."
        return 1
    fi
}


copy_clipboard() {
    local source=$1

    if command -v wl-copy >/dev/null 2>&1; then
        wl-copy < "$source"
    elif command -v xclip >/dev/null 2>&1; then
        xclip -selection clipboard -i < "$source"
    else
        error "Copy mode requires 'wl-copy' or 'xclip'."
        return 1
    fi
}


# Print the numeric IDs represented by <file>.in<ID>, sorted numerically.
# Files such as a.input and a.in-old are deliberately ignored.
saved_input_ids() {
    local file=$1
    local input id

    shopt -s nullglob
    for input in "$file".in[0-9]*; do
        [[ -f $input ]] || continue
        id=${input#"$file.in"}
        [[ $id =~ ^[1-9][0-9]*$ ]] && printf '%s\n' "$id"
    done | sort -n
    shopt -u nullglob
}


next_input_id() {
    local file=$1
    local id last=0

    while IFS= read -r id; do
        (( id > last )) && last=$id
    done < <(saved_input_ids "$file")

    printf '%d' "$(( last + 1 ))"
}


# A single lock covers cases shared by sources with the same canonical stem.
lock_saved_inputs() {
    local file=$1
    local absolute hash lockfile

    require_tool flock || return $?
    require_tool realpath || return $?
    require_tool sha256sum || return $?
    absolute=$(realpath -m -- "$file") || return $?
    hash=$(printf '%s' "$absolute" | sha256sum) || return $?
    hash=${hash%% *}
    mkdir -p -- .rn-build/locks || return $?
    lockfile=".rn-build/locks/${hash}.lock"
    exec {SAVED_LOCK_FD}> "$lockfile" || return $?
    if ! flock -x "$SAVED_LOCK_FD"; then
        exec {SAVED_LOCK_FD}>&-
        error "Could not lock saved inputs for '$file'."
        return 1
    fi
}


unlock_saved_inputs() {
    exec {SAVED_LOCK_FD}>&-
}


require_saved_input() {
    local file=$1
    local id=$2

    [[ $id =~ ^[1-9][0-9]*$ ]] || {
        error "Invalid input ID '$id'. IDs must be positive integers."
        return 1
    }
    [[ -f $file.in$id ]] || {
        error "Saved input ID $id ('$file.in$id') does not exist."
        return 1
    }
}


show_saved_input() {
    local file=$1
    local id=$2
    local input="$file.in$id"
    local modified bytes

    modified=$(date -r "$input" '+%Y-%m-%d %H:%M:%S') || return $?
    bytes=$(stat -c '%s' -- "$input") || return $?

    ui_header "INPUT #$id" "$input" "$orange"
    printf '  %bModified%b  %s (local)\n' "$grey" "$reset" "$modified"
    printf '  %bSize%b      %s bytes\n\n' "$grey" "$reset" "$bytes"
    if [[ -s $input ]]; then
        show_file "$input" || return $?
    else
        printf '%b(empty)%b\n' "$dim$grey" "$reset"
    fi
    ui_footer "$dim$grey"
    printf '\n'
}


valid_duration() {
    [[ $1 =~ ^([0-9]+([.][0-9]+)?|[.][0-9]+)$ && $1 =~ [1-9] ]]
}


# Call with the stem lock held. The temporary file prevents a clipboard failure
# from leaving a partial saved test case.
snapshot_clipboard() {
    local file=$1
    local id destination

    id=$(next_input_id "$file")
    destination="$file.in$id"
    paste_clipboard_atomic "$destination" || return $?

    SNAPSHOT_ID=$id
    SNAPSHOT_FILE=$destination
}


# ── Compilation ───────────────────────────────────────────────────────────────

resolve_source() {
    local requested=$1
    local basename=${requested##*/}

    case $basename in
        *.cpp|*.py|*.c|*.java|*.rs|*.go|*.kt)
            RESOLVED_SOURCE=$requested
            RESOLVED_STEM=${requested%.*}
            RESOLVED_LANGUAGE=${requested##*.}
            ;;
        *.*)
            error "Unsupported source extension in '$requested'. Use .cpp, .py, .c, .java, .rs, .go, or .kt."
            return 1
            ;;
        *)
            RESOLVED_SOURCE=$requested.cpp
            RESOLVED_STEM=$requested
            RESOLVED_LANGUAGE=cpp
            ;;
    esac
}


source_candidate_score() {
    local requested=$1
    local basename=${requested##*/}

    case $basename in
        *.cpp|*.py|*.c|*.java|*.rs|*.go|*.kt)
            if [[ -f $requested ]]; then
                SOURCE_CANDIDATE_SCORE=40
            else
                SOURCE_CANDIDATE_SCORE=20
            fi
            ;;
        *.*)
            SOURCE_CANDIDATE_SCORE=-1
            ;;
        *)
            if [[ -f $requested.cpp ]]; then
                SOURCE_CANDIDATE_SCORE=30
            else
                SOURCE_CANDIDATE_SCORE=10
            fi
            ;;
    esac
}


require_tool() {
    local tool=$1
    if [[ $tool == */* ]]; then
        [[ -x $tool ]] && return 0
    else
        command -v "$tool" >/dev/null 2>&1 && return 0
    fi
    error "Required tool '$tool' was not found or is not executable."
    return 1
}


# Keep non-C++ artifacts distinct even when sources share a basename.
artifact_path() {
    local source=$1
    local absolute hash

    require_tool realpath || return $?
    require_tool sha256sum || return $?
    absolute=$(realpath -m -- "$source") || return $?
    hash=$(printf '%s' "$absolute" | sha256sum) || return $?
    hash=${hash%% *}
    printf '.rn-build/%s.%s' "${source##*/}" "${hash:0:16}"
}


build_command() {
    local source_file=$1
    local language=$2
    local output=$3
    local mode=standard
    local tool status package main_class output_dir
    local -a compiler_command=()

    [[ -f $source_file ]] || {
        error "Source file '$source_file' does not exist."
        return 1
    }

    PREPARED_COMMAND=()
    (( DEBUG_BUILD )) && mode=debug

    case $language in
        py)
            require_tool "$PYTHON" || return $?
            PREPARED_COMMAND=("$PYTHON")
            (( DEBUG_BUILD )) && PREPARED_COMMAND+=(-X dev)
            PREPARED_COMMAND+=("$source_file")
            return 0
            ;;
        cpp)
            tool=$CXX
            compiler_command=("$CXX")
            if (( DEBUG_BUILD )); then
                compiler_command+=("${DEBUG_FLAGS[@]}")
            else
                compiler_command+=("${STANDARD_FLAGS[@]}")
            fi
            compiler_command+=(-o "$output" "$source_file")
            PREPARED_COMMAND=("$(executable_path "$output")")
            ;;
        c)
            tool=$CC
            output_dir=$output
            compiler_command=("$CC")
            if (( DEBUG_BUILD )); then
                compiler_command+=("${C_DEBUG_FLAGS[@]}")
            else
                compiler_command+=("${C_FLAGS[@]}")
            fi
            compiler_command+=(-o "$output/program" "$source_file")
            PREPARED_COMMAND=("$(executable_path "$output/program")")
            ;;
        rs)
            tool=$RUSTC
            output_dir=$output
            compiler_command=("$RUSTC" --edition=2021)
            (( DEBUG_BUILD )) && compiler_command+=(-C debuginfo=2 -C debug-assertions=yes -C overflow-checks=yes)
            compiler_command+=(-o "$output/program" "$source_file")
            PREPARED_COMMAND=("$(executable_path "$output/program")")
            ;;
        go)
            tool=$GO
            output_dir=$output
            compiler_command=("$GO" build)
            (( DEBUG_BUILD )) && compiler_command+=(-race '-gcflags=all=-N -l')
            compiler_command+=(-o "$output/program" "$source_file")
            PREPARED_COMMAND=("$(executable_path "$output/program")")
            ;;
        java)
            tool=$JAVAC
            require_tool "$JAVA" || return $?
            output_dir=$output/classes
            compiler_command=("$JAVAC")
            (( DEBUG_BUILD )) && compiler_command+=(-g -Xlint:all)
            compiler_command+=(-d "$output/classes" "$source_file")
            package=$(sed -nE 's/^[[:space:]]*package[[:space:]]+([A-Za-z_][A-Za-z_0-9.]*)[[:space:]]*;.*/\1/p' "$source_file" | head -n 1)
            main_class=${source_file##*/}
            main_class=${main_class%.java}
            [[ -n $package ]] && main_class=$package.$main_class
            PREPARED_COMMAND=("$JAVA")
            (( DEBUG_BUILD )) && PREPARED_COMMAND+=(-ea)
            PREPARED_COMMAND+=(-cp "$output/classes" "$main_class")
            ;;
        kt)
            tool=$KOTLINC
            require_tool "$JAVA" || return $?
            output_dir=$output
            compiler_command=("$KOTLINC")
            (( DEBUG_BUILD )) && compiler_command+=(-Xdebug -Xassertions=jvm)
            compiler_command+=("$source_file" -include-runtime -d "$output/program.jar")
            PREPARED_COMMAND=("$JAVA")
            (( DEBUG_BUILD )) && PREPARED_COMMAND+=(-ea)
            PREPARED_COMMAND+=(-jar "$output/program.jar")
            ;;
        *)
            error "Internal error: unsupported language '$language'."
            return 1
            ;;
    esac

    require_tool "$tool" || return $?
    [[ -z $output_dir ]] || mkdir -p -- "$output_dir" || return $?
    ui_header 'BUILD' "$source_file · $mode" "$grey" "$dim"
    printf '  %bCompiler%b  %s\n' "$grey" "$reset" "$tool"
    "${compiler_command[@]}"
    status=$?

    if (( status != 0 )); then
        printf '  %b%b✗ Compilation failed%b (exit %d)\n' \
            "$red" "$bold" "$reset" "$status" >&2
        ui_footer "$dim$grey" >&2
        return "$status"
    fi

    printf '  %b✓ Compilation finished%b\n' "$grey" "$reset"
    ui_footer "$dim$grey"
}


compile() {
    local output=$file

    if [[ $language != cpp && $language != py ]]; then
        output=$(artifact_path "$source") || return $?
    fi
    build_command "$source" "$language" "$output" || return $?
    RUN_COMMAND=("${PREPARED_COMMAND[@]}")
}


compile_debug() {
    local previous=$DEBUG_BUILD
    local status

    DEBUG_BUILD=1
    compile "$1"
    status=$?
    DEBUG_BUILD=$previous
    return "$status"
}


# ── Execution and resource reporting ─────────────────────────────────────────

executable_path() {
    if [[ $1 == /* ]]; then
        printf '%s' "$1"
    else
        printf './%s' "$1"
    fi
}


format_memory() {
    local kib=${1:-0}
    local whole decimal

    if [[ ! $kib =~ ^[0-9]+$ ]]; then
        printf '?'
    elif (( kib >= 1048576 )); then
        whole=$(( kib / 1048576 ))
        decimal=$(( (kib % 1048576) * 10 / 1048576 ))
        printf '%d.%d GiB' "$whole" "$decimal"
    elif (( kib >= 1024 )); then
        whole=$(( kib / 1024 ))
        decimal=$(( (kib % 1024) * 10 / 1024 ))
        printf '%d.%d MiB' "$whole" "$decimal"
    else
        printf '%d KiB' "$kib"
    fi
}


# Render one row of the resource table. Values wrap inside the second column on
# narrow terminals instead of pushing the right border out of alignment.
resource_row() {
    local label=$1
    local value=$2
    local value_style="$dim$silver"
    local border_style="$dim$grey"
    local label_style="$dim$grey"
    local width label_width=13 value_width chunk

    (( $# >= 3 )) && value_style=$3

    width=$(terminal_width)
    value_width=$(( width - label_width - 7 ))

    while :; do
        chunk=${value:0:value_width}
        value=${value:value_width}

        printf '%b│%b %b' "$border_style" "$reset" "$label_style"
        pad_right "$label" "$label_width"
        printf '%b %b│%b %b' "$reset" "$border_style" "$reset" "$value_style"
        pad_right "$chunk" "$value_width"
        printf '%b %b│%b\n' "$reset" "$border_style" "$reset"

        label=''
        [[ -n $value ]] || break
    done
}


resource_separator() {
    local width label_width=13 value_width
    width=$(terminal_width)
    value_width=$(( width - label_width - 7 ))

    printf '%b├' "$dim$grey"
    repeat_char '─' "$(( label_width + 2 ))"
    printf '┼'
    repeat_char '─' "$(( value_width + 2 ))"
    printf '┤%b\n' "$reset"
}


resource_bottom() {
    local width label_width=13 value_width
    width=$(terminal_width)
    value_width=$(( width - label_width - 7 ))

    printf '%b└' "$dim$grey"
    repeat_char '─' "$(( label_width + 2 ))"
    printf '┴'
    repeat_char '─' "$(( value_width + 2 ))"
    printf '┘%b\n' "$reset"
}


show_resources() {
    local elapsed=$1
    local user_time=$2
    local system_time=$3
    local cpu=$4
    local peak_kib=$5
    local status=$6

    local status_color status_text memory
    memory=$(format_memory "$peak_kib")

    if (( status == 0 )); then
        status_color=$green
        status_text="✓ Success (exit 0)"
    else
        status_color=$red
        status_text="✗ Failed (exit $status)"
    fi

    {
        box_top 'RESOURCE USAGE' '' "$dim$grey" "$dim$grey"
        resource_row 'Status' "$status_text" "$dim$status_color"
        resource_separator
        resource_row 'Wall time' "$elapsed s" "$dim$silver"
        resource_row 'CPU time' "$user_time s user · $system_time s system · $cpu" "$dim$silver"
        resource_row 'Peak memory' "$memory ($peak_kib KiB)" "$dim$silver"
        resource_bottom
    } >&2
}


# GNU time writes machine-readable metrics to a temporary file. This prevents
# timing text from being mixed with the program's stdout or stderr and lets us
# render all measurements together after execution.
run_timed() {
    local input=${2:-}
    local capture=${3:-}
    local time_limit=${4:-}
    local timing_file timing_line status
    local elapsed user_time system_time cpu peak_kib
    local capture_fifo tee_pid tee_status
    local -a command

    command=("${RUN_COMMAND[@]}")

    RUN_TIMED_CAPTURE_STATUS=0
    [[ -n $capture ]] && RUN_TIMED_CAPTURE_STATUS=1

    if [[ -n $time_limit ]]; then
        command=(timeout --foreground --kill-after=1s -- "$time_limit" "${RUN_COMMAND[@]}")
    fi

    if [[ $TIME_BIN == */* ]]; then
        [[ -x $TIME_BIN ]] || {
            error "GNU time was not found at '$TIME_BIN'."
            return 1
        }
    elif ! command -v "$TIME_BIN" >/dev/null; then
        error "GNU time command '$TIME_BIN' was not found."
        return 1
    fi

    timing_file=$(mktemp "${TMPDIR:-/tmp}/cp-run-time.XXXXXX") || {
        error 'Could not create the timing file.'
        return 1
    }

    if [[ -n $input ]]; then
        "$TIME_BIN" \
            -o "$timing_file" \
            -f '%e|%U|%S|%P|%M' \
            "${command[@]}" < "$input"
        status=$?
    elif [[ -n $capture ]]; then
        capture_fifo="$capture.pipe"
        if ! mkfifo -- "$capture_fifo"; then
            rm -f -- "$timing_file"
            RUN_TIMED_CAPTURE_STATUS=1
            error "Could not create a pipe for interactive input."
            return 1
        fi

        # Write to the capture before forwarding each chunk to the program.
        # Stop the reader when the program exits so Ctrl+D is not required.
        tee -- "$capture_fifo" <&0 > "$capture" &
        tee_pid=$!
        "$TIME_BIN" \
            -o "$timing_file" \
            -f '%e|%U|%S|%P|%M' \
            "${command[@]}" < "$capture_fifo"
        status=$?

        kill "$tee_pid" 2>/dev/null || true
        wait "$tee_pid" 2>/dev/null
        tee_status=$?
        rm -f -- "$capture_fifo"
        RUN_TIMED_CAPTURE_STATUS=0
        if (( tee_status != 0 && tee_status != 141 && tee_status != 143 )); then
            RUN_TIMED_CAPTURE_STATUS=$tee_status
        fi
    else
        "$TIME_BIN" \
            -o "$timing_file" \
            -f '%e|%U|%S|%P|%M' \
            "${command[@]}"
        status=$?
    fi

    timing_line=$(tail -n 1 -- "$timing_file" 2>/dev/null || true)
    rm -f -- "$timing_file"

    IFS='|' read -r elapsed user_time system_time cpu peak_kib \
        <<< "$timing_line"

    elapsed=${elapsed:-?}
    user_time=${user_time:-?}
    system_time=${system_time:-?}
    cpu=${cpu:-?}
    peak_kib=${peak_kib:-?}

    # Start the resource panel cleanly even if the program omitted its final \n.
    printf '\n' >&2
    show_resources "$elapsed" "$user_time" "$system_time" "$cpu" "$peak_kib" "$status"

    return "$status"
}


run_input() {
    local file=$1
    local input=$2

    ui_header 'INPUT' "$input" "$orange"
    show_file "$input" || return $?
    printf '\n'

    ui_header 'OUTPUT' '' "$green"
    run_timed "$file" "$input"
}


run_output_file() {
    local file=$1
    local input=$2
    local output=$3
    local temporary status

    if [[ -n $input ]]; then
        ui_header 'INPUT' "$input" "$orange"
        show_file "$input" || return $?
        printf '\n'
    else
        ui_header 'RUN' 'interactive input' "$green"
    fi

    ui_header 'OUTPUT' "saving to $output" "$green"
    temporary=$(mktemp "$output.tmp.XXXXXX") || {
        error "Could not create a temporary file beside '$output'."
        return 1
    }
    if [[ -n $input ]]; then
        run_timed "$file" "$input" > "$temporary"
    else
        run_timed "$file" > "$temporary"
    fi
    status=$?
    if (( status != 0 )); then
        rm -f -- "$temporary"
        return "$status"
    fi
    if ! mv -- "$temporary" "$output"; then
        rm -f -- "$temporary"
        error "Could not save output '$output'."
        return 1
    fi
    printf '  %bSaved output%b  %s\n' "$green" "$reset" "$output"
}


# Compile once, then run the requested saved inputs in the given order. A
# failing test case does not prevent the remaining cases from running.
run_saved_inputs() {
    local file=$1
    shift

    local id status=0 case_status

    compile "$file" || return $?

    for id in "$@"; do
        printf '\n'
        run_input "$file" "$file.in$id"
        case_status=$?
        (( case_status != 0 )) && status=$case_status
    done

    return "$status"
}


# Each generated case is checked against the brute-force program. Preserve the
# first failing input using the normal saved-input numbering for easy replay.
run_stress() (
    local file=$1 brute_source=$2 generator_source=$3
    local limit=${STRESS_LIMIT:-1000}
    local case_timeout=${STRESS_TIMEOUT:-2}
    local stress_dir input_file actual_file expected_file
    local case_no generator_status actual_status expected_status id
    local -a main_command brute_command generator_command

    [[ $limit =~ ^[1-9][0-9]*$ ]] || {
        error "STRESS_LIMIT must be a positive integer."
        return 1
    }
    valid_duration "$case_timeout" || {
        error "STRESS_TIMEOUT must be a positive number of seconds."
        return 1
    }
    command -v timeout >/dev/null 2>&1 || {
        error "Stress mode requires GNU 'timeout'."
        return 1
    }
    compile "$file" || return $?
    main_command=("${RUN_COMMAND[@]}")
    stress_dir=$(mktemp -d "${TMPDIR:-/tmp}/cp-stress.XXXXXX") || {
        error 'Could not create a temporary stress directory.'
        return 1
    }
    trap 'rm -rf -- "$stress_dir"' EXIT

    resolve_source "$brute_source" || return $?
    brute_source=$RESOLVED_SOURCE
    build_command "$brute_source" "$RESOLVED_LANGUAGE" "$stress_dir/brute" || return $?
    brute_command=("${PREPARED_COMMAND[@]}")

    resolve_source "$generator_source" || return $?
    generator_source=$RESOLVED_SOURCE
    build_command "$generator_source" "$RESOLVED_LANGUAGE" "$stress_dir/generator" || return $?
    generator_command=("${PREPARED_COMMAND[@]}")

    input_file="$stress_dir/input"
    actual_file="$stress_dir/actual"
    expected_file="$stress_dir/expected"
    ui_header 'STRESS' "$limit cases · ${case_timeout}s per program" "$cyan"

    for (( case_no = 1; case_no <= limit; case_no++ )); do
        timeout --kill-after=1s -- "$case_timeout" \
            "${generator_command[@]}" "$case_no" > "$input_file"
        generator_status=$?
        if (( generator_status != 0 )); then
            error "Generator failed on case $case_no (exit $generator_status)."
            return "$generator_status"
        fi

        timeout --kill-after=1s -- "$case_timeout" \
            "${main_command[@]}" < "$input_file" > "$actual_file"
        actual_status=$?
        timeout --kill-after=1s -- "$case_timeout" \
            "${brute_command[@]}" < "$input_file" > "$expected_file"
        expected_status=$?

        if (( actual_status != 0 || expected_status != 0 )) || \
            ! cmp -s -- "$expected_file" "$actual_file"; then
            lock_saved_inputs "$file" || return $?
            id=$(next_input_id "$file")
            cp -- "$input_file" "$file.in$id" || {
                unlock_saved_inputs
                error "Could not save failing input '$file.in$id'."
                return 1
            }
            unlock_saved_inputs
            printf '  %bMismatch on case %d%b · saved %s.in%s\n' \
                "$red" "$case_no" "$reset" "$file" "$id"
            printf '  Program exit: %d · brute exit: %d\n' \
                "$actual_status" "$expected_status"
            diff -u --label "brute ($brute_source)" \
                --label "program ($source)" \
                "$expected_file" "$actual_file" || true
            return 1
        fi

        if (( case_no % 100 == 0 || case_no == limit )); then
            printf '  Passed %d/%d case(s)\n' "$case_no" "$limit"
        fi
    done
)


print_help() {
    cat <<'HELP'
Competitive Programming Runner

USAGE
  ./run [--debug] <source> [command] [arguments]
  ./run [--debug] <command> <source> [arguments]
                                  Source and command may appear in either order
  ./run --source <source> [command] [arguments]
                                  Name the source explicitly when operands are ambiguous
  ./run <source>                   Build if needed, then run interactively
  ./run <file> -s                  Run interactively and save input to next <file>.in<ID>
  ./run <file> -i                  Run using <file>.in1
  ./run <file> -p                  Paste into <file>.in1 and run
  ./run <file> -pa                 Paste into the next <file>.in<ID> and run
  ./run <file> -o <output>         Run interactively and save stdout
  ./run <file> --out <output>      Same as -o
  ./run <file> -c                  Build only (unsupported for Python)
  ./run <file> -d                  Debug build only (unsupported for Python)
  ./run <file> -f <inputs...>      Run named input files (also --in)
  ./run <file> -f <inputs...> -o <outputs...>
                                  Run each input into its matching output file
  ./run <file> --all               Run all saved inputs in ID order
  ./run <file> --save              Run interactively and save input
  ./run <file> --id <IDs...>       Run selected saved input IDs
  ./run <file> --list              Show saved IDs, timestamps, sizes, and contents
  ./run <file> --show <ID>         Show one saved input and its metadata
  ./run <file> --copy <ID>         Copy one saved input to the clipboard
  ./run <file> --delete <ID>       Remove one saved input
  ./run <file> --last              Run the most recently modified saved input
  ./run <file> --diff <ID> <file>  Compare stdout with an expected output file
  ./run <file> --timeout <secs>    Run interactively with a time limit
  ./run <file> --stress <brute-source> <generator-source>
                                  Compare generated cases; languages may be mixed
  ./run <file> --clear             Remove all saved inputs for this source
  --debug                         May appear anywhere
  --source <source>               May appear anywhere; use once
  ./run --help                     Show this help

  For --stress with the command first, put the main source immediately after
  the command. Short commands require a leading dash, such as -p and -i.

EXAMPLES
  ./run a
  ./run -i a
  ./run --debug -i a.py
  ./run --show 3 a
  ./run -f a first.in second.in
  ./run a -o answer.out
  ./run a -p -o answer.out
  ./run a -i --out saved-answer.out
  ./run --out answer.out a
  ./run -o answer.out --source a
  ./run a --in first.in second.in --out first.out second.out
  ./run --stress a brute.cpp generator.py
  ./run a.py --all
  ./run a.kt --stress brute.rs generator.py
  ./run a --debug
  ./run a --all --debug
  ./run --debug a --id 1,2
  ./run a --save
  ./run a -pa
  ./run a --all
  ./run a --id 1,2,3
  ./run a --id 1, 2, 3
  ./run a --list
  ./run a --show 3
  ./run a --copy 3
  ./run a --delete 3
  ./run a --last
  ./run a --diff 3 expected.out
  ./run a --timeout 2
  ./run a --stress brute.cpp generator.cpp
  ./run a --clear

INPUT AND OUTPUT FILES
  -f and --in are aliases. -o and --out are aliases. Use -o/--out alone with
  interactive input, or combine it with -i, -p, or -pa for one input file.
  With -f/--in, provide one output file per input; files are paired in order.
  Program stdout goes to each output file and runner status stays in the
  terminal. Existing output files are replaced only after successful runs.
  An output cannot overwrite the source, an input, or another output in the
  same command. If several listed files look like sources, put the main source
  before the file options or name it with --source. Outputs cannot overwrite
  the C++ binary, .rn-build artifacts, or an existing source file.

SOURCES
  Supported extensions: .cpp, .py, .c, .java, .rs, .go, .kt.
  A name without an extension always uses .cpp, even if another source with
  the same stem exists. Saved inputs are shared by stem across languages.
  Sources are single-file programs using standard language tooling. A Java
  entry class must match its filename and may have a package declaration.
  Build artifacts for non-C++ compiled languages are kept in .rn-build/.

SAVED INPUTS
  The ID is the numeric suffix of the input file:
      a.in1  -> ID 1
      a.in2  -> ID 2

  `-pa` uses max(existing ID) + 1, so deleting an older case never changes or
  renumbers the remaining IDs. `-s`/`--save` uses the same next ID for typed
  interactive input. `--last` uses modification time (highest ID breaks ties).
  Both commas and spaces are accepted by --id.
  Concurrent saves and deletions of a stem are serialized with flock.

DEBUG BUILD
  C++ builds define LOCAL, so local debug headers can be included conditionally.
  --debug uses checks appropriate to the source language. C/C++ use extended
  warnings and sanitizers; Python uses development mode; Rust enables debug
  assertions and overflow checks; Go enables race detection; Java and Kotlin
  enable assertions. The -d command is a debug build-only shortcut for compiled
  languages. Python does not support -c or -d. File-management commands accept
  --debug without building.

STRESS TESTING
  The generator receives the 1-based case number as its first argument and
  prints input to stdout. The first failing input is saved as the next .in<ID>.
  Source extensions may be mixed; bare helper names mean .cpp. Output is
  compared byte for byte. By default, 1000 cases are checked, with
  two seconds allowed for each generator, program, and brute-force run.
  STRESS_LIMIT=500 and STRESS_TIMEOUT=5 override those defaults.

ENVIRONMENT
  CXX=clang++ ./run a              Use another compiler
  CC=clang ./run a.c               Use another C compiler
  PYTHON=pypy3 ./run a.py          Use another Python interpreter
  JAVAC=/path/to/javac ./run A.java
  JAVA=/path/to/java ./run A.java
  RUSTC=/path/to/rustc ./run a.rs
  GO=/path/to/go ./run a.go
  KOTLINC=/path/to/kotlinc ./run a.kt
  TIME_BIN=gtime ./run a --all     Override GNU time's path
  NO_COLOR=1 ./run a -i            Disable terminal colors
HELP
}


# ── Main ──────────────────────────────────────────────────────────────────────

DEBUG_BUILD=0
option=''
option_position=-1
explicit_source=''
source_is_explicit=0
expect_source=0
positionals=()
groups=()
current_group=free
input_seen=0
output_seen=0
for argument in "$@"; do
    if (( expect_source )); then
        if [[ -z $argument || $argument == -* ]]; then
            error "Option '--source' requires a source file name."
            exit 1
        fi
        explicit_source=$argument
        expect_source=0
        continue
    fi
    case $argument in
        --debug)
            DEBUG_BUILD=1
            ;;
        --source)
            if (( source_is_explicit )); then
                error "Option '--source' may be used only once."
                exit 1
            fi
            source_is_explicit=1
            expect_source=1
            ;;
        -f|--in)
            if [[ -n $option ]] || (( input_seen )); then
                error "Option '$argument' cannot be combined with another input or command option."
                exit 1
            fi
            input_seen=1
            current_group=in
            ;;
        -o|--out)
            if (( output_seen )) ||
               { [[ -n $option ]] && [[ $option != -i && $option != -p && $option != -pa ]]; }; then
                error "Option '$argument' cannot be combined with another output or command option."
                exit 1
            fi
            output_seen=1
            current_group=out
            ;;
        -s|--save|-i|-p|-pa|-c|-d|--all|--id|--list|--show|--copy|--delete|--last|--diff|--timeout|--stress|--clear|--help|-h)
            if [[ -n $option ]] || (( input_seen )) ||
               { (( output_seen )) && [[ $argument != -i && $argument != -p && $argument != -pa ]]; }; then
                error "Choose one command option; '$argument' cannot be combined with the other command options."
                exit 1
            fi
            option=$argument
            option_position=${#positionals[@]}
            ;;
        -*)
            error "Unknown option '$argument'."
            exit 1
            ;;
        *)
            positionals+=("$argument")
            groups+=("$current_group")
            ;;
    esac
done

if (( expect_source )); then
    error "Option '--source' requires a source file name."
    exit 1
fi

if [[ $option == --help || $option == -h ]]; then
    print_help
    exit 0
fi

if (( ${#positionals[@]} == 0 && ! source_is_explicit )); then
    print_help >&2
    exit 1
fi

if (( input_seen || output_seen )); then
    free_indexes=()
    input_indexes=()
    output_indexes=()
    for index in "${!positionals[@]}"; do
        case ${groups[index]} in
            free) free_indexes+=("$index") ;;
            in) input_indexes+=("$index") ;;
            out) output_indexes+=("$index") ;;
        esac
    done

    if (( source_is_explicit )); then
        if (( ${#free_indexes[@]} > 0 )); then
            error 'A source was supplied both positionally and with --source.'
            exit 1
        fi
        source_index=-1
    elif (( ${#free_indexes[@]} > 1 )); then
        error 'Specify exactly one source outside the -f/--in and -o/--out file lists.'
        exit 1
    elif (( ${#free_indexes[@]} == 1 )); then
        source_index=${free_indexes[0]}
    else
        if (( input_seen && ! output_seen && ${#input_indexes[@]} < 2 )); then
            error 'Provide a source and at least one input file after -f/--in.'
            exit 1
        fi
        if (( output_seen && ! input_seen && ${#output_indexes[@]} < 2 )); then
            error 'Provide a source and one output file after -o/--out.'
            exit 1
        fi
        candidates=()
        if (( input_seen && output_seen )); then
            if (( ${#input_indexes[@]} == ${#output_indexes[@]} + 1 )); then
                candidates=("${input_indexes[@]}")
            elif (( ${#output_indexes[@]} == ${#input_indexes[@]} + 1 )); then
                candidates=("${output_indexes[@]}")
            else
                error 'Specify one source and an equal number of input and output files.'
                exit 1
            fi
        elif (( input_seen )); then
            candidates=("${input_indexes[@]}")
        else
            candidates=("${output_indexes[@]}")
        fi

        (( ${#candidates[@]} > 0 )) || {
            error 'No source file was provided.'
            exit 1
        }
        source_index=${candidates[0]}
        best_score=-2
        plausible_sources=0
        for index in "${candidates[@]}"; do
            source_candidate_score "${positionals[index]}"
            (( SOURCE_CANDIDATE_SCORE >= 30 )) && (( plausible_sources += 1 ))
            if (( SOURCE_CANDIDATE_SCORE > best_score )); then
                best_score=$SOURCE_CANDIDATE_SCORE
                source_index=$index
            fi
        done
        if (( plausible_sources > 1 )); then
            error 'Several files could be the source. Put the main source before -f/--in or -o/--out.'
            exit 1
        fi
    fi

    if (( source_is_explicit )); then
        resolve_source "$explicit_source" || exit $?
    else
        resolve_source "${positionals[source_index]}" || exit $?
    fi
    source=$RESOLVED_SOURCE
    file=$RESOLVED_STEM
    language=$RESOLVED_LANGUAGE
    input_files=()
    output_files=()
    for index in "${!positionals[@]}"; do
        (( index == source_index )) && continue
        case ${groups[index]} in
            in) input_files+=("${positionals[index]}") ;;
            out) output_files+=("${positionals[index]}") ;;
        esac
    done

    if (( input_seen && ${#input_files[@]} == 0 )); then
        error 'No input files were provided after -f/--in.'
        exit 1
    fi
    if (( output_seen && ${#output_files[@]} == 0 )); then
        error 'No output files were provided after -o/--out.'
        exit 1
    fi
    if (( input_seen && output_seen && ${#input_files[@]} != ${#output_files[@]} )); then
        error "Input/output file counts must match (${#input_files[@]} input, ${#output_files[@]} output)."
        exit 1
    fi
    if (( output_seen && ! input_seen && ${#output_files[@]} != 1 )); then
        error 'Use exactly one output file when no -f/--in files are provided.'
        exit 1
    fi

    if (( output_seen )); then
        require_tool realpath || exit $?
        source_absolute=$(realpath -m -- "$source") || exit $?
        build_root_absolute=$(realpath -m -- .rn-build) || exit $?
        binary_absolute=''
        if [[ $language == cpp ]]; then
            binary_absolute=$(realpath -m -- "$file") || exit $?
        fi
        safety_inputs=("${input_files[@]}")
        case $option in
            -i|-p) safety_inputs+=("$file.in1") ;;
            -pa)
                lock_saved_inputs "$file" || exit $?
                safety_inputs+=("$file.in$(next_input_id "$file")")
                ;;
        esac
        output_absolutes=()
        for output in "${output_files[@]}"; do
            output_absolute=$(realpath -m -- "$output") || exit $?
            if [[ $output_absolute == "$source_absolute" ]]; then
                error "Output '$output' would overwrite the source file."
                exit 1
            fi
            if [[ -n $binary_absolute && $output_absolute == "$binary_absolute" ]]; then
                error "Output '$output' would overwrite the C++ binary."
                exit 1
            fi
            if [[ $output_absolute == "$build_root_absolute" ||
                  $output_absolute == "$build_root_absolute"/* ]]; then
                error "Output '$output' cannot be saved inside .rn-build."
                exit 1
            fi
            if [[ -d $output_absolute ]]; then
                error "Output '$output' is a directory."
                exit 1
            fi
            if [[ -f $output_absolute ]]; then
                case ${output_absolute##*/} in
                    *.cpp|*.py|*.c|*.java|*.rs|*.go|*.kt)
                        error "Output '$output' would overwrite an existing source file."
                        exit 1
                        ;;
                esac
            fi
            for input in "${safety_inputs[@]}"; do
                if [[ $output_absolute == "$(realpath -m -- "$input")" ]]; then
                    error "Output '$output' would overwrite input '$input'."
                    exit 1
                fi
            done
            for previous in "${output_absolutes[@]}"; do
                if [[ $output_absolute == "$previous" ]]; then
                    error "Output '$output' is listed more than once."
                    exit 1
                fi
            done
            output_absolutes+=("$output_absolute")
        done
    fi

    case $option in
        -i)
            input="$file.in1"
            [[ -f $input ]] || {
                error "Input file '$input' does not exist."
                exit 1
            }
            ;;
        -p)
            input="$file.in1"
            lock_saved_inputs "$file" || exit $?
            paste_clipboard_atomic "$input" || exit $?
            unlock_saved_inputs
            ;;
        -pa)
            snapshot_clipboard "$file" || exit $?
            unlock_saved_inputs
            input=$SNAPSHOT_FILE
            printf '  %bSaved input #%s%b  %s\n' \
                "$grey" "$SNAPSHOT_ID" "$reset" "$SNAPSHOT_FILE"
            ;;
    esac

    compile "$file" || exit $?
    if [[ $option == -i || $option == -p || $option == -pa ]]; then
        printf '\n'
        run_output_file "$file" "$input" "${output_files[0]}"
        exit $?
    fi
    if (( input_seen )); then
        status=0
        for index in "${!input_files[@]}"; do
            input=${input_files[index]}
            printf '\n'
            if [[ ! -f $input ]]; then
                error "Input file '$input' does not exist; skipping it."
                status=1
                continue
            fi
            if (( output_seen )); then
                run_output_file "$file" "$input" "${output_files[index]}"
            else
                run_input "$file" "$input"
            fi
            case_status=$?
            (( case_status != 0 )) && status=$case_status
        done
        exit "$status"
    fi

    printf '\n'
    run_output_file "$file" '' "${output_files[0]}"
    exit $?
fi

# The source may precede or follow the command. When an option has source-like
# operands (notably -f and --stress), command-first syntax puts the main source
# immediately after the option: --stress main brute generator.
source_index=0
if (( source_is_explicit )); then
    source_index=-1
elif (( option_position == 0 )); then
    case $option in
        --show|--copy|--delete|--timeout)
            if (( ${#positionals[@]} == 2 )); then
                if [[ $option == --timeout ]]; then
                    valid_duration "${positionals[0]}" && source_index=1
                elif [[ ${positionals[0]} =~ ^[1-9][0-9]*$ ]]; then
                    source_index=1
                fi
            fi
            ;;
        --id)
            for index in "${!positionals[@]}"; do
                if [[ ! ${positionals[index]} =~ ^[0-9,]+$ ]]; then
                    source_index=$index
                    break
                fi
            done
            ;;
        --diff)
            if (( ${#positionals[@]} == 3 )) &&
               [[ ${positionals[0]} =~ ^[1-9][0-9]*$ ]]; then
                source_index=2
            fi
            ;;
    esac
fi

if (( source_is_explicit )); then
    resolve_source "$explicit_source" || exit $?
else
    resolve_source "${positionals[source_index]}" || exit $?
fi
source=$RESOLVED_SOURCE
file=$RESOLVED_STEM
language=$RESOLVED_LANGUAGE
command_args=()
for index in "${!positionals[@]}"; do
    (( index == source_index )) || command_args+=("${positionals[index]}")
done
set -- "${command_args[@]}"

if [[ -z $option ]]; then
    (( $# == 0 )) || {
        error "Expected one source and one command option, such as '-i'."
        exit 1
    }
    compile "$file" || exit $?
    printf '\n'
    ui_header 'RUN' 'interactive input' "$green"
    run_timed "$file"
    exit $?
fi

case "$option" in
    -s|--save)
        (( $# == 0 )) || {
            error "Option '$option' does not accept arguments."
            exit 1
        }

        lock_saved_inputs "$file" || exit $?
        compile "$file" || exit $?
        id=$(next_input_id "$file")
        input="$file.in$id"
        temporary=$(mktemp "$input.tmp.XXXXXX") || {
            error "Could not create a temporary input beside '$input'."
            exit 1
        }
        trap 'rm -f -- "$temporary"' EXIT

        printf '\n'
        ui_header 'RUN' "interactive input · saving to $input" "$green"
        run_timed "$file" '' "$temporary"
        status=$?

        if (( RUN_TIMED_CAPTURE_STATUS != 0 )); then
            error "Could not capture interactive input for '$input'."
            exit "$RUN_TIMED_CAPTURE_STATUS"
        fi

        mv -- "$temporary" "$input" || {
            error "Could not save '$input'."
            exit 1
        }
        unlock_saved_inputs
        printf '  %bSaved input #%s%b  %s\n' \
            "$grey" "$id" "$reset" "$input"
        exit "$status"
        ;;

    -i)
        input="$file.in1"
        [[ -f $input ]] || {
            error "Input file '$input' does not exist."
            exit 1
        }

        compile "$file" && {
            printf '\n'
            run_input "$file" "$input"
        }
        ;;

    -p)
        input="$file.in1"
        lock_saved_inputs "$file" || exit $?
        paste_clipboard_atomic "$input" || exit $?
        unlock_saved_inputs

        compile "$file" && {
            printf '\n'
            run_input "$file" "$input"
        }
        ;;

    -pa)
        lock_saved_inputs "$file" || exit $?
        snapshot_clipboard "$file" || exit $?
        unlock_saved_inputs
        printf '  %bSaved input #%s%b  %s\n' \
            "$grey" "$SNAPSHOT_ID" "$reset" "$SNAPSHOT_FILE"

        compile "$file" && {
            printf '\n'
            run_input "$file" "$SNAPSHOT_FILE"
        }
        ;;

    -c)
        if [[ $language == py ]]; then
            error "Option '-c' is unsupported for Python; run '$source' directly."
            exit 1
        fi
        compile "$file"
        ;;

    -d)
        if [[ $language == py ]]; then
            error "Option '-d' is unsupported for Python; use --debug with a run command."
            exit 1
        fi
        compile_debug "$file"
        ;;

    --all)
        (( $# == 0 )) || {
            error "Option '--all' does not accept arguments."
            exit 1
        }

        mapfile -t ids < <(saved_input_ids "$file")
        (( ${#ids[@]} > 0 )) || {
            error "No saved inputs exist for '$source'."
            exit 1
        }

        run_saved_inputs "$file" "${ids[@]}"
        ;;

    --id)
        (( $# > 0 )) || {
            error "No IDs were provided. Example: $0 $file --id 1,2,3"
            exit 1
        }

        raw_ids=$*
        raw_ids=${raw_ids//,/ }
        read -r -a ids <<< "$raw_ids"

        (( ${#ids[@]} > 0 )) || {
            error 'No input IDs were provided.'
            exit 1
        }

        declare -A seen_ids=()
        selected_ids=()
        for id in "${ids[@]}"; do
            [[ $id =~ ^[1-9][0-9]*$ ]] || {
                error "Invalid input ID '$id'. IDs must be positive integers."
                exit 1
            }
            [[ -f $file.in$id ]] || {
                error "Saved input ID $id ('$file.in$id') does not exist."
                exit 1
            }
            [[ -n ${seen_ids[$id]:-} ]] && continue
            seen_ids[$id]=1
            selected_ids+=("$id")
        done

        run_saved_inputs "$file" "${selected_ids[@]}"
        ;;

    --list)
        (( $# == 0 )) || {
            error "Option '--list' does not accept arguments."
            exit 1
        }

        mapfile -t ids < <(saved_input_ids "$file")
        if (( ${#ids[@]} == 0 )); then
            printf '  %bNo saved inputs%b for %s\n' "$grey" "$reset" "$source"
            exit 0
        fi

        for id in "${ids[@]}"; do
            show_saved_input "$file" "$id" || exit $?
        done
        ;;

    --show)
        (( $# == 1 )) || {
            error "Usage: $0 $file --show <ID>"
            exit 1
        }
        id=$1
        require_saved_input "$file" "$id" || exit $?
        show_saved_input "$file" "$id"
        ;;

    --copy)
        (( $# == 1 )) || {
            error "Usage: $0 $file --copy <ID>"
            exit 1
        }
        id=$1
        require_saved_input "$file" "$id" || exit $?
        input="$file.in$id"
        copy_clipboard "$input" || exit $?
        printf '  %bCopied%b %s to the clipboard\n' "$green" "$reset" "$input"
        ;;

    --delete)
        (( $# == 1 )) || {
            error "Usage: $0 $file --delete <ID>"
            exit 1
        }
        id=$1
        lock_saved_inputs "$file" || exit $?
        require_saved_input "$file" "$id" || exit $?
        input="$file.in$id"
        rm -- "$input" || exit $?
        unlock_saved_inputs
        printf '  %bDeleted%b %s\n' "$grey" "$reset" "$input"
        ;;

    --last)
        (( $# == 0 )) || {
            error "Option '--last' does not accept arguments."
            exit 1
        }

        mapfile -t ids < <(saved_input_ids "$file")
        (( ${#ids[@]} > 0 )) || {
            error "No saved inputs exist for '$source'."
            exit 1
        }

        last_id=''
        last_modified=''
        for id in "${ids[@]}"; do
            modified=$(stat -c '%y' -- "$file.in$id") || exit $?
            if [[ -z $last_id || $modified > $last_modified ]] || \
                { [[ $modified == "$last_modified" ]] && (( id > last_id )); }; then
                last_id=$id
                last_modified=$modified
            fi
        done

        printf '  %bMost recent input%b  %s.in%s\n' \
            "$grey" "$reset" "$file" "$last_id"
        run_saved_inputs "$file" "$last_id"
        ;;

    --diff)
        (( $# == 2 )) || {
            error "Usage: $0 $file --diff <ID> <expected-output>"
            exit 1
        }
        id=$1
        expected=$2
        require_saved_input "$file" "$id" || exit $?
        [[ -f $expected ]] || {
            error "Expected output file '$expected' does not exist."
            exit 1
        }

        compile "$file" || exit $?
        actual=$(mktemp "${TMPDIR:-/tmp}/cp-run-actual.XXXXXX") || {
            error 'Could not create a temporary output file.'
            exit 1
        }
        trap 'rm -f -- "$actual"' EXIT

        input="$file.in$id"
        printf '\n'
        ui_header 'DIFF' "$input · $expected" "$cyan"
        run_timed "$file" "$input" > "$actual"
        status=$?
        if (( status != 0 )); then
            error "Program exited with status $status; comparison skipped."
            exit "$status"
        fi

        diff -u --label "expected ($expected)" \
            --label "actual ($input)" "$expected" "$actual"
        status=$?
        if (( status == 0 )); then
            printf '  %b✓ Output matches%b %s\n' "$green" "$reset" "$expected"
        elif (( status == 1 )); then
            printf '  %b✗ Output differs%b from %s\n' "$red" "$reset" "$expected"
        fi
        exit "$status"
        ;;

    --timeout)
        (( $# == 1 )) || {
            error "Usage: $0 $file --timeout <seconds>"
            exit 1
        }
        duration=$1
        valid_duration "$duration" || {
            error "Timeout must be a positive number of seconds."
            exit 1
        }
        command -v timeout >/dev/null 2>&1 || {
            error "Timeout mode requires GNU 'timeout'."
            exit 1
        }

        compile "$file" || exit $?
        printf '\n'
        ui_header 'RUN' "interactive input · ${duration}s limit" "$green"
        run_timed "$file" '' '' "$duration"
        status=$?
        if (( status == 124 || status == 137 )); then
            error "Program timed out after $duration second(s)."
        fi
        exit "$status"
        ;;

    --stress)
        (( $# == 2 )) || {
            error "Usage: $0 $source --stress <brute-source> <generator-source>"
            exit 1
        }
        run_stress "$file" "$1" "$2"
        ;;

    --clear)
        (( $# == 0 )) || {
            error "Option '--clear' does not accept arguments."
            exit 1
        }

        lock_saved_inputs "$file" || exit $?
        mapfile -t ids < <(saved_input_ids "$file")
        if (( ${#ids[@]} == 0 )); then
            unlock_saved_inputs
            printf '  %bNo saved inputs%b for %s\n' "$grey" "$reset" "$source"
            exit 0
        fi

        for id in "${ids[@]}"; do
            rm -f -- "$file.in$id" || exit $?
        done
        unlock_saved_inputs
        printf '  %bCleared %d saved input(s)%b for %s:\n' \
            "$grey" "${#ids[@]}" "$reset" "$source"
        for id in "${ids[@]}"; do
            printf '    %s\n' "$file.in$id"
        done
        ;;

    --help|-h)
        print_help
        ;;

    *)
        error "Unknown option '$option'."
        printf "Run '%s --help' to see every supported command.\n" "$0" >&2
        exit 1
        ;;
esac
