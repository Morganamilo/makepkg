known_hash_algos=({ck,md5,sha{1,224,256,384,512},b2})

base_pkgbuild_vars=(arch backup changelog checkdepends conflicts depends
                        groups epoch install license makedepends noextract
                        optdepends options pkgbase pkgdesc pkgname pkgrel pkgver provides
                        replaces source url validpgpkeys "${known_hash_algos[@]/%/sums}")

pkgbuild_functions=(pkgver verify prepare build check package)

pkgbuild_vars=( "${base_pkgbuild_vars[@]}" )

conf_vars=(DLAGENTS VCSCLIENTS CARCH CHOST CPPFLAGS CFLAGS CXXFLAGS RUSTFLAGS LDFLAGS
           LTOFLAGS MAKEFLAGS DEBUG_CFLAGS DEBUG_CXXFLAGS DEBUG_RUSTFLAGS BUILDENV
           DISTCC_HOSTS BUILDDIR GPGKEY OPTIONS INTEGRITY_CHECK STRIP_BINARIES
           STRIP_SHARED STRIP_STATIC MAN_DIRS DOC_DIRS PURGE_TARGETS DBGSRCDIR
           PKGDEST SRCDEST SRCPKGDEST LOGDEST PACKAGER COMPRESSGZ COMPRESSBZ2
           COMPRESSXZ COMPRESSZST COMPRESSLRZ COMPRESSLZO COMPRESSZ COMPRESSLZ4 COMPRESSLZ
           PKGEXT SRCEXT PACMAN_AUTH)

readonly -a known_hash_algos pkgbuild_functions base_pkgbuild_vars conf_vars

msg2() {
	:
}

msg() {
	:
}

source_safe() {
	local file="$1"
	local shellopts=$(shopt -p extglob)
	shopt -u extglob

	if ! source "$1"; then
		exit 1
	fi

	eval "$shellopts"
}

cd_safe() {
	if ! cd "$1"; then
		exit 1
	fi
}

escape() {
	local val="$1"
	val="${val//\\/\\\\}"
	val="${val//\"/\\\"}"
	val="${val//$'\n'/\\\\n}"
	printf -- "%s" "$val"
}

expand_pkgbuild_vars() {
	local a arch_type

	if [[ $(typeof_var arch) == ARRAY ]]; then
		for a in "${arch[@]}"; do
			pkgbuild_vars+=( "${base_pkgbuild_vars[@]/%/_$a}" )
		done
	fi

	readonly -a pkgbuild_vars
}

typeof_var() {
	local type=$(declare -p "$1" 2>/dev/null)

	if [[ "$type" == "declare --"* ]]; then
		printf "STRING"
	elif [[ "$type" == "declare -a"* ]]; then
		printf "ARRAY"
	elif [[ "$type" == "declare -A"* ]]; then
		printf "MAP"
	else
		printf "NONE"
	fi
}

dump_string() {
	local varname=$1 prefix="$2"
	local val="$(escape "${!varname}")"

	printf -- '%s STRING %s "%s"\n' "$prefix" "$varname" "$val"

}

dump_array() {
	local val varname=$1 prefix="$2"
	local arr=$varname[@]

	printf -- '%s ARRAY %s' "$prefix" "$varname"

	for val in "${!arr}"; do
		val="$(escape "$val")"
		printf -- ' "%s"' "$val"
	done

	printf '\n'
}

dump_map() {
	local key varname=$1 prefix="$2"
	declare -n map=$varname

	printf -- '%s MAP %s' "$prefix" "$varname"

	for key in "${!map[@]}"; do
		val="${map[$key]}"

		key="$(escape "$key")"
		val="$(escape "$val")"

		printf -- ' "%s" "%s"' "$key" "$val"
	done

	printf '\n'
}

dump_var() {
	local varname=$1 prefix="${2:-"VAR GLOBAL"}"
	local type=$(typeof_var $varname)

	if [[ $type == STRING ]]; then
		dump_string $varname "$prefix"
	elif [[ $type == ARRAY ]]; then
		dump_array $varname "$prefix"
	elif [[ $type == MAP ]]; then
		dump_map $varname "$prefix"
	fi
}

grep_function() {
	local funcname=$1 regex="$2"

	declare -f $funcname 2>/dev/null | grep -E "$regex"
}

dump_function_vars() {
	local funcname=$1 varname attr_regex decl new_vars
	declare -A new_vars
	printf -v attr_regex '^[[:space:]]* [a-z1-9_]*\+?='

	if ! have_function $funcname; then
		return
	fi

	# this function requires extglob - save current status to restore later
	local shellopts=$(shopt -p extglob)
	shopt -s extglob

	while read -r; do
		# strip leading whitespace and any usage of declare
		decl=${REPLY##*([[:space:]])}
		varname=${decl%%[+=]*}

		local -I $varname
		new_vars[$varname]=1
		eval "$decl"
	done < <(grep_function "$funcname" "$attr_regex")

	for varname in "${pkgbuild_vars[@]}"; do
		if [[ -v "new_vars[$varname]" ]]; then
			dump_var $varname "VAR FUNCTION $funcname"
		fi
	done

	eval "$shellopts"
}

dump_functions_vars() {
	local name

	dump_function_vars package

	for name in "${pkgname[@]}"; do
		dump_function_vars package_${name}
	done
}


dump_global_vars() {
	local varname

	for varname in "${pkgbuild_vars[@]}"; do
		dump_var $varname
	done
}

have_function() {
	declare -f "$1" >/dev/null
}

dump_function_name() {
	local funcname=$1

	if have_function $funcname; then
		printf -- "FUNCTION %s\n" $funcname
	fi
}

dump_function_names() {
	local name funcname

	for funcname in "${pkgbuild_functions[@]}"; do
		dump_function_name $funcname
	done

	for name in "${pkgname[@]}"; do
		dump_function_name package_${name}
	done
}

dump_pkgbuild() {
	source_safe "$1"

	expand_pkgbuild_vars
	dump_global_vars
	dump_functions_vars
	dump_function_names
}

run_function() {
	local pkgfunc="$1"
	local workingdir="$2"
	local ret=0

	if [[ ! -z $PKGNAME ]]; then
		pkgname=$PKGNAME
		unset PKGNAME
	fi

	cd_safe "$workingdir"
	"$pkgfunc"
}

dump_config() {
	local varname file

	for file in "$@"; do
		source_safe "$file"
	done

	for varname in "${conf_vars[@]}"; do
		dump_var $varname "VAR CONFIG"
	done
}

run_function_safe() {
	source_safe "$1"

	local -
	shopt -o -s errexit errtrace

	trap "exit 1" ERR
	run_function "$2" "$3"
}

# usage:
# pkgbuild dump <path/to/pkgbuild>
# pkgbuild conf <path/to/config/files>...
# pkgbuild run <function-name> <workingdir>

if [[ "$1" == dump ]]; then
	shift
	dump_pkgbuild "$@"
elif [[ "$1" == conf ]]; then
	shift
	dump_config "$@"
elif [[ "$1" == run ]]; then
	shift
	run_function_safe "$@"
fi

exit 0
