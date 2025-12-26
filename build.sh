#!/bin/bash

cargobin=`which cargo`
if [ -z "$cargobin" ]; then
	echo "rust not installed, install for current user with rustup.rs ..."
	
	export RUSTUP_DIST_SERVER="https://rsproxy.cn"
	export RUSTUP_UPDATE_ROOT="https://rsproxy.cn/rustup"	
	curl --proto '=https' --tlsv1.2 https://sh.rustup.rs >> .xrustup.sh
	sh .xrustup.sh -y

	if [ $? -ne  0 ]; then
		echo "fail to install rust !!!"
		exit 127
	fi
	mkdir -p $HOME/.cargo
cat > $HOME/.cargo/config <<EOF
[source.crates-io]
replace-with = 'rsproxy-sparse'
[source.rsproxy]
registry = "https://rsproxy.cn/crates.io-index"
[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
[registries.rsproxy]
index = "https://rsproxy.cn/crates.io-index"
[net]
git-fetch-with-cli = true
EOF

	source "$HOME/.cargo/env"
	rm -rf .xrustup.sh
fi

yum install -y clang

cargo build --all --release
