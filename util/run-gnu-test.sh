#!/usr/bin/env bash

# 准备工具MAKE READLINK
MAKE=$(command -v gmake||command -v make)
READLINK=$(command -v greadlink||command -v readlink)

# 当前脚本位置
ME_dir="$(dirname -- "$("${READLINK}" -fm -- "$0")")"
# syskits的根目录
REPO_main_dir="$(dirname -- "${ME_dir}")"

echo "ME_dir='${ME_dir}'"
echo "REPO_main_dir='${REPO_main_dir}'"

set -e

# gnu目录必须和syskits目录平级
path_SYSKITS=${path_SYSKITS:-${REPO_main_dir}}
path_GNU="$("${READLINK}" -fm -- "${path_GNU:-${path_SYSKITS}/../gnu}")"

echo "path_SYSKITS='${path_SYSKITS}'"
echo "path_GNU='${path_GNU}'"

# 并行编译设置（加速）
NPROC=$(command -v ${path_GNU}/src/nproc||command -v nproc)
MAKEFLAGS="${MAKEFLAGS} -j ${NPROC}"
export MAKEFLAGS

# 进入gnu目录
cd "${path_GNU}" && echo "[ pwd:'${PWD}' ]"

export RUST_BACKTRACE=1

## 麻烦的两类测试：TTY测试和SELINUX测试

# 判断传入的测试名中是否包含selinux测试
has_selinux_tests=false
if test $# -ge 1; then
    for t in "$@"; do
        if [[ "$t" == *"selinux"* ]]; then
                has_selinux_tests=true
                break
        fi
    done
fi

# 跑需要TTY的测试
if [[ "$1" == "run-tty" ]]; then
    # Handle TTY tests - dynamically find tests requiring TTY and run each individually
    shift
    TTY_TESTS=$(grep -r "require_controlling_input_terminal" tests --include="*.sh" --include="*.pl" -l 2>/dev/null)
    echo "Running TTY tests individually:"
    # If a test fails, it can break the implementation of the other tty tests. By running them separately this stops the different tests from being able to break each other
    for test in $TTY_TESTS; do
        echo "  Running: $test"
        script -qec "timeout -sKILL 5m '${MAKE}' check TESTS='$test' SUBDIRS=. RUN_EXPENSIVE_TESTS=no VERBOSE=no gl_public_submodule_commit='' srcdir='${path_GNU}'" /dev/null || :
    done
    exit 0
# run-root分支（几乎用不到）
elif [[ "$1" == "run-root" && "$has_selinux_tests" == true ]]; then
    # Handle SELinux root tests separately
    shift
    if test -n "$CI"; then
        echo "Running SELinux tests as root"
        # Don't use check-root here as the upstream root tests is hardcoded
        sudo "${MAKE}" check TESTS="$*" SUBDIRS=. RUN_EXPENSIVE_TESTS=no RUN_VERY_EXPENSIVE_TESTS=no VERBOSE=no gl_public_submodule_commit="" srcdir="${path_GNU}" TEST_SUITE_LOG="tests/test-suite-root.log" || :
    fi
    exit 0
# 常用分支
elif test "$1" != "run-root" && test "$1" != "run-tty"; then
    if test $# -ge 1; then
        SPECIFIC_TESTS=""
        for t in "$@"; do

            # Construct the full path
            full_path="$path_GNU/$t"

            # Check if the file exists with .sh, .pl extension or without any extension in the $path_GNU directory
            if [ -f "$full_path" ] || [ -f "$full_path.sh" ] || [ -f "$full_path.pl" ]; then
                SPECIFIC_TESTS="$SPECIFIC_TESTS $t"
            else
                echo "Error: Test file $full_path, $full_path.sh, or $full_path.pl does not exist!"
                exit 1
            fi
        done
        # 去掉多余空格
        SPECIFIC_TESTS=$(echo "$SPECIFIC_TESTS" | xargs)
        echo "Running specific tests: $SPECIFIC_TESTS"
    fi
fi

# * timeout used to kill occasionally errant/"stuck" processes (note: 'release' testing takes ~1 hour; 'debug' testing takes ~2.5 hours)
# * `gl_public_submodule_commit=` disables testing for use of a "public" gnulib commit (which will fail when using shallow gnulib checkouts)
# * `srcdir=..` specifies the GNU source directory for tests (fixing failing/confused 'tests/factor/tNN.sh' tests and causing no harm to other tests)
#shellcheck disable=SC2086

if test "$1" != "run-root" && test "$1" != "run-tty"; then
    # run the regular tests
    if test $# -ge 1; then
        # 真正跑测试的地方
        timeout -sKILL 4h "${MAKE}" check TESTS="$SPECIFIC_TESTS" SUBDIRS=. RUN_EXPENSIVE_TESTS=no RUN_VERY_EXPENSIVE_TESTS=no VERBOSE=no gl_public_submodule_commit="" srcdir="${path_GNU}" || : # Kill after 4 hours in case something gets stuck in make
    else
        timeout -sKILL 4h "${MAKE}" check SUBDIRS=. RUN_EXPENSIVE_TESTS=no RUN_VERY_EXPENSIVE_TESTS=no VERBOSE=no gl_public_submodule_commit="" srcdir="${path_GNU}" || : # Kill after 4 hours in case something gets stuck in make
    fi
else
    # in case we would like to run tests requiring root
    if test -z "$1" -o "$1" == "run-root"; then
        if test -n "$CI"; then
            if test $# -ge 2; then
                echo "Running check-root to run only root tests"
                sudo "${MAKE}" check-root TESTS="$2" SUBDIRS=. RUN_EXPENSIVE_TESTS=no RUN_VERY_EXPENSIVE_TESTS=no VERBOSE=no gl_public_submodule_commit="" srcdir="${path_GNU}" TEST_SUITE_LOG="tests/test-suite-root.log" || :
            else
                echo "Running check-root to run only root tests"
                sudo "${MAKE}" check-root SUBDIRS=. RUN_EXPENSIVE_TESTS=no RUN_VERY_EXPENSIVE_TESTS=no VERBOSE=no gl_public_submodule_commit="" srcdir="${path_GNU}" TEST_SUITE_LOG="tests/test-suite-root.log" || :
            fi
        fi
    fi
fi
