#needsrootforbuild
%global __cargo_skip_build 0
%global syskit_install_src target/release
%global syskit_app_bin /usr/local/syskits
%global kits_name syskits
%global _debugsource_packages 1
%global _debuginfo_subpackages 1
%define _unpackaged_files_terminate_build 0
%define debug_package %{nil}

Name:           syskits
Version:        2.1.1
Release:        dev.1
Summary:        CTyunOs sysKits is suit of programes to replease coreutils.

License:        Mulan PSL v2
URL:            https://ctyun.cn
Source0:        %{name}-v%{version}.tar.gz

ExclusiveArch:  x86_64 aarch64

BuildRequires:  gcc clang openssl-libs libselinux-devel libacl-devel

%description
CTyunOs sysKits is suit of programes to release coreutils.

Summary:        %{summary}

%prep
%autosetup -n %{name}

%build
sh build.sh

%install
mkdir -p %{buildroot}/usr/bin
install -m 755 %{syskit_install_src}/%{kits_name} %{buildroot}/usr/bin
mkdir -p %{buildroot}/%{syskit_app_bin}

%files
%defattr(-,root,root,-)
/usr/bin/syskits
%dir %{syskit_app_bin}

%post
# Define the file list variable
file_list="arch base32 base64 basename basenc cat chcon chgrp chmod chown chroot cksum comm cp csplit split \
du yes whoami who wc vdir dir ls sort mkdir mkfifo mknod mktemp mv nice nohup nproc date df dircolors pwd \
groups hostname numfmt readlink rmdir sleep sum sync touch true truncate expand expr false pr printenv printf \
tsort tty uname unexpand uniq unlink uptime users cut fmt logname echo env dirname"

# Create symbolic links during post installation
for file in $file_list; do
  if [ ! -e %{syskit_app_bin}/$file ]; then
    ln -s /usr/bin/syskits %{syskit_app_bin}/$file
  fi
done

%preun
# Remove symbolic links during pre-uninstallation
if [ $1 -eq 0 ]; then
  rm -rf %{syskit_app_bin}/*
fi

%changelog
* Tue Dec 12 2023 Leon Wang <wangl29@chinatelecom.cn> - 1.1.1-1
- 完成 ctcore 框架开发，syskits实现统一管理操作系统命令能力。
- syskits支持替换关键组件（arch, base32, base64, basename, basenc, cat, chcon, chgrp, chmod, chown, chroot, cksum, comm, cp, csplit, split, du, yes, whoami, who, wc, vdir, dir, ls, sort），且与GNU命令兼容性逐步达到60%以上。
- 可在系统中安装部署，可代替原生Linux系统中GNU coreutils中涉及的Linux命令。
