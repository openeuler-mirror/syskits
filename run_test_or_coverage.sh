#!/bin/bash

# 检查 rust 是否安装
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
cat > $HOME/.cargo/config.toml <<EOF
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

# 安装基本依赖
yum install -y clang libacl-devel libselinux-devel

if [ "$1" = "test" ]; then
    cargo test --workspace
elif [ "$1" = "coverage" ]; then
    echo "Running coverage test..."
    
    # 安装测试相关依赖
    yum install -y libacl-devel libselinux-devel
    
    # 安装测试覆盖率插件
    cargo install cargo-llvm-cov
    
    # 执行测试
    cargo llvm-cov --no-report --all
    
    # 生成lcov.info
    cargo llvm-cov report --lcov --output-path lcov.info
    
    # 生成hmtl报告
    if [ $2 = "report" ]; then
        cargo llvm-cov report --html --output-dir cover_report
    fi
    
    echo "Coverage test completed. Check cover_report directory for results."
else
    echo "no cmd param."
fi
