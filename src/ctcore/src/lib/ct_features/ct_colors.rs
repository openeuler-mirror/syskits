/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

/// The keywords COLOR, OPTIONS, and EIGHTBIT (honored by the
/// slackware version of dircolors) are recognized but ignored.
/// Global config options can be specified before TERM or COLORTERM entries
/// below are TERM or COLORTERM entries, which can be glob patterns, which
/// restrict following config to systems with matching environment variables.
pub static CT_TERMS: &[&str] = &[
    "Eterm",
    "ansi",
    "*color*",
    "con[0-9]*x[0-9]*",
    "cons25",
    "console",
    "cygwin",
    "*direct*",
    "dtterm",
    "gnome",
    "hurd",
    "jfbterm",
    "konsole",
    "kterm",
    "linux",
    "linux-c",
    "mlterm",
    "putty",
    "rxvt*",
    "screen*",
    "st",
    "terminator",
    "tmux*",
    "vt100",
    "xterm*",
];

/// Below are the color init strings for the basic file types.
/// One can use codes for 256 or more colors supported by modern terminals.
/// The default color codes use the capabilities of an 8 color terminal
/// with some additional attributes as per the following codes:
/// Attribute codes:
/// 00=none 01=bold 04=underscore 05=blink 07=reverse 08=concealed
/// Text color codes:
/// 30=black 31=red 32=green 33=yellow 34=blue 35=magenta 36=cyan 37=white
/// Background color codes:
/// 40=black 41=red 42=green 43=yellow 44=blue 45=magenta 46=cyan 47=white
/// #NORMAL 00 /// no color code at all
/// #FILE 00 /// regular file: use no color at all
pub static CT_FILE_TYPES: &[(&str, &str, &str)] = &[
    ("RESET", "rs", "0"),          //RESET (rs)：重置颜色和样式到默认状态。
    ("DIR", "di", "01;34"),        // DIR (di)：目录，以粗体蓝色显示。
    ("LINK", "ln", "01;36"),       // LINK (ln)：符号链接（软链接），以粗体青色显示。
    ("MULTIHARDLINK", "mh", "00"), // MULTIHARDLINK (mh)：拥有多个硬链接的常规文件，不改变颜色（默认颜色）。
    ("FIFO", "pi", "40;33"),       // FIFO (pi)：命名管道（FIFO），以黄色背景和红色前景显示。
    ("SOCK", "so", "01;35"),       // SOCK (so)：套接字，以粗体紫色显示。
    ("DOOR", "do", "01;35"),       // DOOR (do)：门（Solaris特有的文件类型），以粗体紫色显示。
    ("BLK", "bd", "40;33;01"),     // BLK (bd)：块设备驱动，以黄色背景、红色前景和粗体显示。
    ("CHR", "cd", "40;33;01"),     // CHR (cd)：字符设备驱动，以黄色背景、红色前景和粗体显示。
    ("ORPHAN", "or", "40;31;01"), // ORPHAN (or)：指向不存在文件的符号链接，或无法通过stat获取信息的文件，以粗体红底黑字显示。
    ("MISSING", "mi", "00"), // MISSING (mi)：与ORPHAN对应的文件（即被ORPHAN链接指向的文件），不改变颜色（默认颜色）。
    ("SETUID", "su", "37;41"), // SETUID (su)：设置了SUID位的文件（用户ID执行位），以白色背景和红色前景显示。
    ("SETGID", "sg", "30;43"), // SETGID (sg)：设置了SGID位的文件（组ID执行位），以绿色背景和黄色前景显示。
    ("CAPABILITY", "ca", "00"), // CAPABILITY (ca)：具有Linux能力集的文件，不改变颜色（默认颜色）。
    ("STICKY_OTHER_WRITABLE", "tw", "30;42"), // STICKY_OTHER_WRITABLE (tw)：具有粘滞位（+t）且其他用户可写（o+w）的目录，以绿色背景和蓝色前景显示。
    ("OTHER_WRITABLE", "ow", "34;42"), // OTHER_WRITABLE (ow)：其他用户可写（o+w）但不具有粘滞位的目录，以蓝色前景和绿色背景显示。
    ("STICKY", "st", "37;44"), // STICKY (st)：具有粘滞位（+t）但不可其他用户写入的目录，以白色背景和蓝色前景显示。
    ("EXEC", "ex", "01;32"),   // EXEC (ex)：具有执行权限的文件，以粗体绿色显示。
];

/// Colors for file types
///
/// List any file extensions like '.gz' or '.tar' that you would like ls
/// to color below. Put the extension, a space, and the color init string.
/// (and any comments you want to add after a '#')
pub static CT_FILE_COLORS: &[(&str, &str)] = &[
    /*
    // 可执行文件
    (".cmd", "01;32"),
    (".exe", "01;32"),
    (".com", "01;32"),
    (".btm", "01;32"),
    (".bat", "01;32"),
    (".sh", "01;32"),
    (".csh", "01;32"),*/
    // 归档文件或压缩文件
    (".tar", "01;31"),
    (".tgz", "01;31"),
    (".arc", "01;31"),
    (".arj", "01;31"),
    (".taz", "01;31"),
    (".lha", "01;31"),
    (".lz4", "01;31"),
    (".lzh", "01;31"),
    (".lzma", "01;31"),
    (".tlz", "01;31"),
    (".txz", "01;31"),
    (".tzo", "01;31"),
    (".t7z", "01;31"),
    (".zip", "01;31"),
    (".z", "01;31"),
    (".dz", "01;31"),
    (".gz", "01;31"),
    (".lrz", "01;31"),
    (".lz", "01;31"),
    (".lzo", "01;31"),
    (".xz", "01;31"),
    (".zst", "01;31"),
    (".tzst", "01;31"),
    (".bz2", "01;31"),
    (".bz", "01;31"),
    (".tbz", "01;31"),
    (".tbz2", "01;31"),
    (".tz", "01;31"),
    (".deb", "01;31"),
    (".rpm", "01;31"),
    (".jar", "01;31"),
    (".war", "01;31"),
    (".ear", "01;31"),
    (".sar", "01;31"),
    (".rar", "01;31"),
    (".alz", "01;31"),
    (".ace", "01;31"),
    (".zoo", "01;31"),
    (".cpio", "01;31"),
    (".7z", "01;31"),
    (".rz", "01;31"),
    (".cab", "01;31"),
    (".wim", "01;31"),
    (".swm", "01;31"),
    (".dwm", "01;31"),
    (".esd", "01;31"),
    // Image formats
    (".avif", "01;35"),
    (".jpg", "01;35"),
    (".jpeg", "01;35"),
    (".mjpg", "01;35"),
    (".mjpeg", "01;35"),
    (".gif", "01;35"),
    (".bmp", "01;35"),
    (".pbm", "01;35"),
    (".pgm", "01;35"),
    (".ppm", "01;35"),
    (".tga", "01;35"),
    (".xbm", "01;35"),
    (".xpm", "01;35"),
    (".tif", "01;35"),
    (".tiff", "01;35"),
    (".png", "01;35"),
    (".svg", "01;35"),
    (".svgz", "01;35"),
    (".mng", "01;35"),
    (".pcx", "01;35"),
    (".mov", "01;35"),
    (".mpg", "01;35"),
    (".mpeg", "01;35"),
    (".m2v", "01;35"),
    (".mkv", "01;35"),
    (".webm", "01;35"),
    (".webp", "01;35"),
    (".ogm", "01;35"),
    (".mp4", "01;35"),
    (".m4v", "01;35"),
    (".mp4v", "01;35"),
    (".vob", "01;35"),
    (".qt", "01;35"),
    (".nuv", "01;35"),
    (".wmv", "01;35"),
    (".asf", "01;35"),
    (".rm", "01;35"),
    (".rmvb", "01;35"),
    (".flc", "01;35"),
    (".avi", "01;35"),
    (".fli", "01;35"),
    (".flv", "01;35"),
    (".gl", "01;35"),
    (".dl", "01;35"),
    (".xcf", "01;35"),
    (".xwd", "01;35"),
    (".yuv", "01;35"),
    (".cgm", "01;35"),
    (".emf", "01;35"),
    (".ogv", "01;35"),
    (".ogx", "01;35"),
    // 音频格式
    (".aac", "00;36"),
    (".au", "00;36"),
    (".flac", "00;36"),
    (".m4a", "00;36"),
    (".mid", "00;36"),
    (".midi", "00;36"),
    (".mka", "00;36"),
    (".mp3", "00;36"),
    (".mpc", "00;36"),
    (".ogg", "00;36"),
    (".ra", "00;36"),
    (".wav", "00;36"),
    (".oga", "00;36"),
    (".opus", "00;36"),
    (".spx", "00;36"),
    (".xspf", "00;36"),
    // 文件备份
    ("*~", "00;90"),
    ("*#", "00;90"),
    (".bak", "00;90"),
    (".old", "00;90"),
    (".orig", "00;90"),
    (".part", "00;90"),
    (".rej", "00;90"),
    (".swp", "00;90"),
    (".tmp", "00;90"),
    (".dpkg-dist", "00;90"),
    (".dpkg-old", "00;90"),
    (".ucf-dist", "00;90"),
    (".ucf-new", "00;90"),
    (".ucf-old", "00;90"),
    (".rpmnew", "00;90"),
    (".rpmorig", "00;90"),
    (".rpmsave", "00;90"),
];

pub static CT_FILE_ATTRIBUTE_CODES: &[(&str, &str)] = &[
    ("normal", "no"),
    ("norm", "no"),
    ("file", "fi"),
    ("reset", "rs"),
    ("dir", "di"),
    ("lnk", "ln"),
    ("link", "ln"),
    ("symlink", "ln"),
    ("orphan", "or"),
    ("missing", "mi"),
    ("fifo", "pi"),
    ("pipe", "pi"),
    ("sock", "so"),
    ("blk", "bd"),
    ("block", "bd"),
    ("chr", "cd"),
    ("char", "cd"),
    ("door", "do"),
    ("exec", "ex"),
    ("left", "lc"),
    ("leftcode", "lc"),
    ("right", "rc"),
    ("rightcode", "rc"),
    ("end", "ec"),
    ("endcode", "ec"),
    ("suid", "su"),
    ("setuid", "su"),
    ("sgid", "sg"),
    ("setgid", "sg"),
    ("sticky", "st"),
    ("other_writable", "ow"),
    ("owr", "ow"),
    ("sticky_other_writable", "tw"),
    ("owt", "tw"),
    ("capability", "ca"),
    ("multihardlink", "mh"),
    ("clrtoeol", "cl"),
];
