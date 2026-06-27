/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//
// 该Rust函数是一个命令行程序，主要功能是生成和更新一个名为"docs"的目录中的Markdown文件。
// 它通过读取一个名为"docs/tldr.zip"的压缩文件来获取一些信息，并使用这些信息来更新"docs/src/SUMMARY.md"和其他Markdown文件的内容。
// 该程序还使用一个名为"./util/show-utils.sh"的外部脚本来获取一些命令行工具的信息，并将这些信息写入Markdown文件中。

extern crate tool_derive;
use clap::Command;
use ctcore::Tool;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use zip::ZipArchive;
#[derive(tool_derive::Tools)]
struct Ctdoc;

fn main() -> io::Result<()> {
    let mut tldr_zip = File::open("docs/tldr.zip")
        .ok()
        .and_then(|f| ZipArchive::new(f).ok());

    if tldr_zip.is_none() {
        println!("Warning: No tldr archive found, so the documentation will not include examples.");
        println!(
            "To include examples in the documentation, download the tldr archive and put it in the docs/ folder."
        );
        println!();
        println!("  curl https://tldr.sh/assets/tldr.zip -o docs/tldr.zip");
        println!();
    }

    let ct_utils = ALL_COMMANDS.iter().collect();
    match std::fs::create_dir("docs/src/utils/") {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        x => x,
    }?;

    println!("Writing initial info to SUMMARY.md");
    let mut ct_summary = File::create("docs/src/SUMMARY.md")?;

    let _ = write!(
        ct_summary,
        "# Summary\n\
        \n\
        [Introduction](index.md)\n\
        * [Installation](installation.md)\n\
        * [Build from source](build.md)\n\
        * [Platform support](platforms.md)\n\
        * [Contributing](contributing.md)\n\
        * [GNU test coverage](test_coverage.md)\n\
        * [Extensions](extensions.md)\n\
        \n\
        # Reference\n\
        * [Multi-call binary](multicall.md)\n",
    );

    println!("Gathering utils per platform");
    let ct_utils_per_platform = {
        let mut ct_map = HashMap::new();
        for ct_platform in ["unix", "macos", "windows", "unix_android"] {
            let ct_platform_utils: Vec<String> = String::from_utf8(
                std::process::Command::new("./util/show-utils.sh")
                    .arg(format!("--ct_features=feat_os_{}", ct_platform))
                    .output()?
                    .stdout,
            )
            .unwrap()
            .trim()
            .split(' ')
            .map(ToString::to_string)
            .collect();
            ct_map.insert(ct_platform, ct_platform_utils);
        }

        // Linux is a special case because it can support selinux
        let ct_platform_utils: Vec<String> = String::from_utf8(
            std::process::Command::new("./util/show-utils.sh")
                .arg("--ct_features=feat_os_unix feat_selinux")
                .output()?
                .stdout,
        )
        .unwrap()
        .trim()
        .split(' ')
        .map(ToString::to_string)
        .collect();
        ct_map.insert("linux", ct_platform_utils);

        ct_map
    };

    let mut ct_utils = ct_utils.entries().collect::<Vec<_>>();
    ct_utils.sort();

    println!("Writing util per platform table");
    {
        let mut platform_table_file = File::create("docs/src/platform_table.md").unwrap();

        // sum, cksum, b2sum等在所有平台上都可用，但在数据结构中没有列出。
        // 否则，我们会在映射中检查util名称是否存在。
        let check_supported = |name: &str, platform: &str| {
            if name.ends_with("sum") || ct_utils_per_platform[platform].iter().any(|u| u == name) {
                "✓"
            } else {
                " "
            }
        };
        writeln!(
            platform_table_file,
            "| util             | Linux | macOS | Windows | FreeBSD | Android |\n\
             | ---------------- | ----- | ----- | ------- | ------- | ------- |"
        )?;
        for (&name, _) in &ct_utils {
            if name == "[" {
                continue;
            }

            writeln!(
                platform_table_file,
                "| {:<16} | {:<5} | {:<5} | {:<7} | {:<7} | {:<7} |",
                format!("**{name}**"),
                check_supported(name, "linux"),
                check_supported(name, "macos"),
                check_supported(name, "windows"),
                check_supported(name, "unix"),
                check_supported(name, "unix_android"),
            )?;
        }
    }

    println!("Writing to utils");
    for (&ct_name, (_, ct_command)) in ct_utils {
        if ct_name == "[" {
            continue;
        }
        let str = format!("docs/src/utils/{}.md", ct_name);

        let ct_markdown = File::open(format!("src/ct/{ct_name}/{ct_name}.md"))
            .and_then(|mut f: File| {
                let mut s = String::new();
                f.read_to_string(&mut s)?;
                Ok(s)
            })
            .ok();

        if let Ok(f) = File::create(&str) {
            CtMDWriter {
                ct_writebox: Box::new(f),
                ct_command: ct_command(),
                ct_name,
                ct_dr_zip: &mut tldr_zip,
                ct_utils_per_platform: &ct_utils_per_platform,
                ct_markdown,
            }
            .ct_markdown()?;
            println!("Wrote to '{}'", str);
        } else {
            println!("Error writing to {}", str);
        }
        writeln!(ct_summary, "* [{0}](utils/{0}.md)", ct_name)?;
    }
    Ok(())
}

struct CtMDWriter<'a, 'b> {
    ct_writebox: Box<dyn Write>,
    ct_command: Command,
    ct_name: &'a str,
    ct_dr_zip: &'b mut Option<ZipArchive<File>>,
    ct_utils_per_platform: &'b HashMap<&'b str, Vec<String>>,
    ct_markdown: Option<String>,
}

impl<'a, 'b> CtMDWriter<'a, 'b> {
    fn ct_markdown(&mut self) -> io::Result<()> {
        write!(self.ct_writebox, "# {}\n\n", self.ct_name)?;
        self.ct_additional()?;
        // self.ct_usage()?;
        // self.ct_about()?;
        self.ct_options()?;
        // self.ct_after_help()?;
        self.ct_examples()
    }

    fn ct_additional(&mut self) -> io::Result<()> {
        writeln!(self.ct_writebox, "<div class=\"additional\">")?;
        self.ct_platforms()?;
        self.ct_version()?;
        writeln!(self.ct_writebox, "</div>")
    }

    fn ct_platforms(&mut self) -> io::Result<()> {
        writeln!(self.ct_writebox, "<div class=\"platforms\">")?;

        for (ct_feature, ct_icon) in [("linux", "linux"), ("unix", "freebsd")] {
            let ct_feature_stat = self.ct_utils_per_platform[ct_feature]
                .iter()
                .any(|u| u == self.ct_name);
            if self.ct_name.contains("sum") || ct_feature_stat {
                writeln!(
                    self.ct_writebox,
                    "<i class=\"fa fa-brands fa-{}\"></i>",
                    ct_icon
                )?;
            }
        }

        writeln!(self.ct_writebox, "</div>")?;

        Ok(())
    }

    fn ct_version(&mut self) -> io::Result<()> {
        writeln!(
            self.ct_writebox,
            "<div class=\"version\">v{}</div>",
            self.ct_command.render_version().split_once(' ').unwrap().1
        )?;
        Ok(())
    }

    // fn ct_usage(&mut self) -> io::Result<()> {
    //     match &self.ct_markdown {
    //         Some(ct_markdown) => {
    //             let ct_usage_src = cthelp_parser::ct_parse_usage(ct_markdown);
    //             let ct_usage = ct_usage_src.replace("{}", self.ct_name);

    //             writeln!(self.ct_writebox, "\n```")?;
    //             writeln!(self.ct_writebox, "{}", ct_usage)?;
    //             writeln!(self.ct_writebox, "```")
    //         }
    //         None => Ok(()),
    //     }
    // }

    // fn ct_about(&mut self) -> io::Result<()> {
    //     match &self.ct_markdown {
    //         Some(ct_markdown) => {
    //             let ct_about_info = cthelp_parser::ct_parse_about(ct_markdown);
    //             writeln!(self.ct_writebox, "{}", ct_about_info)
    //         }
    //         None => Ok(()),
    //     }

    //     //Ok(())
    // }

    // fn ct_after_help(&mut self) -> io::Result<()> {
    //     match &self.ct_markdown {
    //         Some(ct_markdown) => {
    //             if let Some(ct_after_help) =
    //                 cthelp_parser::ct_parse_section("after help", ct_markdown)
    //             {
    //                 writeln!(self.ct_writebox, "\n\n{}", ct_after_help)?;
    //             }
    //         }
    //         None => {}
    //     }

    //     Ok(())
    // }

    fn ct_examples(&mut self) -> io::Result<()> {
        match self.ct_dr_zip {
            Some(ct_zip) => {
                let ct_content = match ct_get_zip_content(
                    ct_zip,
                    &format!("pages/common/{}.md", self.ct_name),
                ) {
                    Some(f) => f,
                    None => match ct_get_zip_content(
                        ct_zip,
                        &format!("pages/linux/{}.md", self.ct_name),
                    ) {
                        Some(f) => f,
                        None => {
                            println!(
                                "Warning: Could not find tldr examples for page '{}'",
                                self.ct_name
                            );
                            return Ok(());
                        }
                    },
                };

                writeln!(self.ct_writebox, "## Examples")?;
                writeln!(self.ct_writebox)?;
                for ct_line in ct_content
                    .lines()
                    .skip_while(|line_info| !line_info.starts_with('-'))
                {
                    match ct_line.strip_prefix("- ") {
                        Some(l) => writeln!(self.ct_writebox, "{}", l)?,
                        None => {
                            if ct_line.starts_with('`') {
                                writeln!(
                                    self.ct_writebox,
                                    "```shell\n{}\n```",
                                    ct_line.trim_matches('`')
                                )?;
                            } else if ct_line.is_empty() {
                                writeln!(self.ct_writebox)?;
                            } else {
                                println!("Not sure what to do with this line:");
                                println!("{}", ct_line);
                            }
                        }
                    }
                }

                writeln!(self.ct_writebox)?;
                writeln!(
                    self.ct_writebox,
                    "> The examples are provided by the [tldr-pages project](https://tldr.sh) under the [CC BY 4.0 License](https://github.com/tldr-pages/tldr/blob/main/LICENSE.md)."
                )?;
                writeln!(self.ct_writebox, ">")?;
                writeln!(
                    self.ct_writebox,
                    "> Please note that, as ctutils is a work in progress, some examples might fail."
                )?;
            }
            None => {}
        }

        Ok(())
    }

    fn ct_options(&mut self) -> io::Result<()> {
        writeln!(self.ct_writebox, "<h2>Options</h2>")?;
        write!(self.ct_writebox, "<dl>")?;

        for ct_arg in self.ct_command.get_arguments() {
            write!(self.ct_writebox, "<dt>")?;
            let mut first = true;

            // Long aliases
            let ct_long_aliases = match ct_arg.get_long_and_visible_aliases() {
                Some(aliases) => aliases,
                None => Vec::new(),
            };
            for l in ct_long_aliases {
                if first {
                    first = false;
                } else {
                    write!(self.ct_writebox, ", ")?;
                }
                write!(self.ct_writebox, "<code>--{}</code>", l)?;

                // Value names
                if let Some(names) = ct_arg.get_value_names() {
                    let mut formatted_names = String::new();
                    for x in names {
                        formatted_names.push_str(&format!("&lt;{}&gt;", x));
                    }
                    write!(self.ct_writebox, "={}", formatted_names)?;
                }
            }

            // Short aliases
            let ct_short_aliases = match ct_arg.get_short_and_visible_aliases() {
                Some(aliases) => aliases,
                None => Vec::new(),
            };
            for s in ct_short_aliases {
                if first {
                    first = false;
                } else {
                    write!(self.ct_writebox, ", ")?;
                }
                write!(self.ct_writebox, "<code>-{}</code>", s)?;

                // Value names
                if let Some(ct_names) = ct_arg.get_value_names() {
                    let mut formatted_names = String::new();
                    for x in ct_names {
                        formatted_names.push_str(&format!("&lt;{}&gt;", x));
                    }
                    write!(self.ct_writebox, " {}", formatted_names)?;
                }
            }

            writeln!(self.ct_writebox, "</dt>")?;
            let ct_help_info = ct_arg.get_help().unwrap_or_default().to_string();
            let ct_replace_info = ct_help_info.replace('\n', "<br />");
            writeln!(self.ct_writebox, "<dd>\n\n{}</dd>", ct_replace_info)?;
        }

        writeln!(self.ct_writebox, "</dl>\n")
    }
}

fn ct_get_zip_content(
    ct_archive: &mut ZipArchive<impl Read + Seek>,
    ct_name: &str,
) -> Option<String> {
    let mut ct_s = String::new();
    match ct_archive.by_name(ct_name) {
        Ok(mut ct_zip) => {
            if let Err(err) = ct_zip.read_to_string(&mut ct_s) {
                panic!("Failed to read file {}: {}", ct_name, err);
            }
            Some(ct_s)
        }
        Err(err) => {
            panic!("Failed to open file {} in archive: {}", ct_name, err);
        }
    }
}
