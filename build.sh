#!/bin/bash

cargobin=`which cargo`
if [ -z "$cargobin" ]; then
	echo "rust not installed, install for current user with rustup.rs ..."

	curl --proto '=https' --tlsv1.2 https://sh.rustup.rs >> .xrustup.sh
	sh .xrustup.sh -y

	if [ $? -ne  0 ]; then
		echo "fail to install rust !!!"
		exit 127
	fi

cat > $HOME/.cargo/config <<EOF
[source.crates-io]
replace-with = 'sjtu'

[source.sjtu]
registry = "https://mirrors.sjtug.sjtu.edu.cn/git/crates.io-index"
EOF

	source "$HOME/.cargo/env"
	rm -rf .xrustup.sh
fi

cargo build --all --release

