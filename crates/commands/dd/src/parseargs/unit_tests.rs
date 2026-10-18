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

use super::*;

use crate::StatusLevel;
use crate::conversion_tables::*;
use crate::parseargs::Parser;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

#[cfg(target_os = "windows")]
#[allow(clippy::useless_vec)]
#[test]
fn unimplemented_flags_should_error_non_linux() {
    let mut succeeded = Vec::new();

    // The following flags are only implemented in linux
    for flag in [
        "direct",
        "directory",
        "dsync",
        "sync",
        "nonblock",
        "noatime",
        "noctty",
        "nofollow",
    ] {
        let args = vec![format!("iflag={}", flag)];

        if Parser::new()
            .parse(&args.iter().map(AsRef::as_ref).collect::<Vec<_>>()[..])
            .is_ok()
        {
            succeeded.push(format!("iflag={}", flag));
        }

        let args = vec![format!("oflag={}", flag)];

        if Parser::new()
            .parse(&args.iter().map(AsRef::as_ref).collect::<Vec<_>>()[..])
            .is_ok()
        {
            succeeded.push(format!("iflag={}", flag));
        }
    }

    assert!(
        succeeded.is_empty(),
        "The following flags did not panic as expected: {:?}",
        succeeded
    );
}

#[test]
#[allow(clippy::useless_vec)]
fn unimplemented_flags_should_error() {
    let mut succeeded = Vec::new();

    // The following flags are not implemented
    for flag in ["cio", "nolinks", "text", "binary"] {
        let args = vec![format!("iflag={flag}")];

        if Parser::new()
            .parse(&args.iter().map(AsRef::as_ref).collect::<Vec<_>>()[..])
            .is_ok()
        {
            succeeded.push(format!("iflag={flag}"));
        }

        let args = vec![format!("oflag={flag}")];

        if Parser::new()
            .parse(&args.iter().map(AsRef::as_ref).collect::<Vec<_>>()[..])
            .is_ok()
        {
            succeeded.push(format!("iflag={flag}"));
        }
    }

    assert!(
        succeeded.is_empty(),
        "The following flags did not panic as expected: {succeeded:?}"
    );
}

#[test]
fn test_status_level_absent() {
    let args = &["if=foo.file", "of=bar.file"];

    assert_eq!(Parser::new().parse(args).unwrap().status, None);
}

#[test]
fn test_status_level_none() {
    let args = &["status=none", "if=foo.file", "of=bar.file"];

    assert_eq!(
        Parser::new().parse(args).unwrap().status,
        Some(StatusLevel::None)
    );
}

#[test]
fn partial_read_warning_matches_gnu_activation_conditions() {
    assert!(
        Parser::new()
            .parse(&["bs=4", "count=2"])
            .unwrap()
            .warn_partial_read
    );

    for args in [
        &["ibs=4", "obs=4", "count=2"][..],
        &["bs=4", "count=2", "iflag=fullblock"][..],
        &["bs=4", "count=2", "conv=swab"][..],
        &["bs=4", "iflag=count_bytes", "count=1"][..],
    ] {
        assert!(!Parser::new().parse(args).unwrap().warn_partial_read);
    }
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_all_top_level_args_no_leading_dashes() {
    let args = &[
        "if=foo.file",
        "of=bar.file",
        "ibs=10",
        "obs=10",
        "cbs=1",
        "bs=100",
        "count=2",
        "skip=2",
        "seek=2",
        "iseek=2",
        "oseek=2",
        "status=progress",
        "conv=ascii,ucase",
        "iflag=count_bytes,skip_bytes",
        "oflag=append,seek_bytes",
    ];

    let settings = Parser::new().parse(args).unwrap();

    // ibs=10 and obs=10 are overwritten by bs=100
    assert_eq!(settings.ibs, 100);
    assert_eq!(settings.obs, 100);

    // count=2 iflag=count_bytes
    assert_eq!(settings.count, Some(Num::Bytes(2)));

    // seek=2 oflag=seek_bytes
    assert_eq!(settings.seek, 2);

    // skip=2 iflag=skip_bytes
    assert_eq!(settings.skip, 2);

    // status=progress
    assert_eq!(settings.status, Some(StatusLevel::Progress));

    // conv=ascii,ucase
    let ascii_upper = get_ctable(Some(Conversion::Ascii), Some(Case::Upper)).unwrap();
    assert_eq!(
        settings.iconv,
        IConvFlags {
            // ascii implies unblock
            mode: Some(ConversionMode::ConvertThenUnblock(ascii_upper, 1)),
            ..IConvFlags::default()
        },
    );

    // no conv flags apply to output
    assert_eq!(settings.oconv, OConvFlags::default(),);

    // iconv=count_bytes,skip_bytes
    assert_eq!(
        settings.iflags,
        IFlags {
            is_count_bytes: true,
            is_skip_bytes: true,
            ..IFlags::default()
        },
    );

    // oconv=append,seek_bytes
    assert_eq!(
        settings.oflags,
        OFlags {
            is_append: true,
            is_seek_bytes: true,
            ..OFlags::default()
        },
    );
}

#[test]
fn test_status_level_progress() {
    let args = &["if=foo.file", "of=bar.file", "status=progress"];

    let settings = Parser::new().parse(args).unwrap();

    assert_eq!(settings.status, Some(StatusLevel::Progress));
}

#[test]
fn test_status_level_noxfer() {
    let args = &["if=foo.file", "status=noxfer", "of=bar.file"];

    let settings = Parser::new().parse(args).unwrap();

    assert_eq!(settings.status, Some(StatusLevel::Noxfer));
}

#[test]
fn test_status_level_list_uses_last_value() {
    for (value, expected) in [
        ("none,noxfer", StatusLevel::Noxfer),
        ("noxfer,none", StatusLevel::None),
        ("progress,progress", StatusLevel::Progress),
    ] {
        let operand = format!("status={value}");
        let settings = Parser::new().parse(&[&operand]).unwrap();
        assert_eq!(settings.status, Some(expected));
    }
}

#[test]
fn test_status_level_list_rejects_invalid_tokens() {
    for (value, invalid) in [
        ("none,bad", "bad"),
        ("bad,none", "bad"),
        ("none,", ""),
        (",none", ""),
    ] {
        let operand = format!("status={value}");
        let error = Parser::new().parse(&[&operand]).err().unwrap();
        assert_eq!(
            error,
            ParseError::StatusLevelNotRecognized(invalid.to_string())
        );
    }
}

#[test]
fn zero_block_size_operands_are_invalid_numbers() {
    for operand in ["bs=0", "ibs=0", "obs=0", "cbs=0", "bs=0K"] {
        let value = operand.split_once('=').unwrap().1;
        match Parser::new().parse(&[operand]) {
            Err(err) => assert_eq!(err, ParseError::InvalidNumber(value.to_string())),
            Ok(_) => panic!("{operand} should reject zero block size"),
        }
    }
}

#[test]
fn zero_record_count_and_offsets_are_allowed() {
    let args = &["count=0", "skip=0", "seek=0"];
    let settings = Parser::new().parse(args).unwrap();

    assert_eq!(settings.count, Some(Num::Blocks(0)));
    assert_eq!(settings.skip, 0);
    assert_eq!(settings.seek, 0);
}

#[test]
fn test_multiple_flags_options() {
    let args = &[
        "iflag=fullblock,count_bytes",
        "iflag=skip_bytes",
        "oflag=append",
        "oflag=seek_bytes",
        "conv=ascii,ucase",
        "conv=unblock",
        "cbs=512",
    ];
    let settings = Parser::new().parse(args).unwrap();

    // iflag
    assert_eq!(
        settings.iflags,
        IFlags {
            is_fullblock: true,
            is_count_bytes: true,
            is_skip_bytes: true,
            ..Default::default()
        }
    );

    // oflag
    assert_eq!(
        settings.oflags,
        OFlags {
            is_append: true,
            is_seek_bytes: true,
            ..Default::default()
        }
    );

    // conv
    let ascii_upper = get_ctable(Some(Conversion::Ascii), Some(Case::Upper)).unwrap();
    assert_eq!(
        settings.iconv,
        IConvFlags {
            mode: Some(ConversionMode::ConvertThenUnblock(ascii_upper, 512)),
            ..Default::default()
        }
    );
}

#[test]
fn test_override_multiple_options() {
    let args = &[
        "if=foo.file",
        "if=correct.file",
        "of=bar.file",
        "of=correct.file",
        "ibs=256",
        "ibs=1024",
        "obs=256",
        "obs=1024",
        "cbs=1",
        "cbs=2",
        "skip=0",
        "skip=2",
        "seek=0",
        "seek=2",
        "iseek=0",
        "iseek=2",
        "oseek=0",
        "oseek=2",
        "status=none",
        "status=noxfer",
        "count=512",
        "count=1024",
        "iflag=count_bytes",
    ];

    let settings = Parser::new().parse(args).unwrap();

    assert_eq!(settings.infile, Some("correct.file".into()));
    assert_eq!(settings.outfile, Some("correct.file".into()));
    assert_eq!(settings.ibs, 1024);
    assert_eq!(settings.obs, 1024);
    assert_eq!(settings.status, Some(StatusLevel::Noxfer));
    assert_eq!(settings.skip, 2048);
    assert_eq!(settings.seek, 2048);
    assert_eq!(settings.count, Some(Num::Bytes(1024)));
}

#[cfg(unix)]
#[test]
fn non_utf8_input_and_output_paths_are_preserved() {
    let input = OsString::from_vec(b"if=input-\xff".to_vec());
    let output = OsString::from_vec(b"of=output-\xfe".to_vec());
    let settings = Parser::new()
        .parse_os(&[input.as_os_str(), output.as_os_str()])
        .unwrap();

    assert_eq!(
        settings.infile,
        Some(OsString::from_vec(b"input-\xff".to_vec()))
    );
    assert_eq!(
        settings.outfile,
        Some(OsString::from_vec(b"output-\xfe".to_vec()))
    );
}

// // ----- IConvFlags/Output -----

#[test]
fn icf_ctable_error() {
    let args = &["conv=ascii,ebcdic,ibm"];
    assert!(Parser::new().parse(args).is_err());
}

#[test]
fn icf_case_error() {
    let args = &["conv=ucase,lcase"];
    assert!(Parser::new().parse(args).is_err());
}

#[test]
fn icf_block_error() {
    let args = &["cbs=1", "conv=block,unblock"];
    assert!(Parser::new().parse(args).is_err());
}

#[test]
fn block_offset_multiplication_overflow_is_rejected() {
    let value = "9223372036854775807";
    for args in [
        &["ibs=2", "skip=9223372036854775807"][..],
        &["obs=2", "seek=9223372036854775807"][..],
    ] {
        assert_eq!(
            Parser::new().parse(args).err(),
            Some(ParseError::NumberTooLarge(value.to_string()))
        );
    }
}

#[test]
fn block_and_unblock_without_cbs_are_ignored() {
    for conversion in ["block", "unblock", "block,unblock"] {
        let conv = format!("conv={conversion}");
        let settings = Parser::new().parse(&["bs=4", &conv]).unwrap();
        assert_eq!(settings.iconv.mode, None);
        assert!(!settings.buffered);
    }
}

#[test]
fn implied_block_modes_conflict_with_opposite_explicit_modes() {
    for conversion in [
        "ascii,block",
        "block,ascii",
        "ebcdic,unblock",
        "unblock,ebcdic",
        "ibm,unblock",
        "unblock,ibm",
    ] {
        let conv = format!("conv={conversion}");
        let error = Parser::new().parse(&["cbs=2", &conv]).err().unwrap();
        assert_eq!(error, ParseError::MultipleBlockUnblock);
    }
}

#[test]
fn implied_block_modes_allow_matching_explicit_modes() {
    for conversion in ["ascii,unblock", "ebcdic,block", "ibm,block"] {
        let conv = format!("conv={conversion}");
        assert!(Parser::new().parse(&["cbs=2", &conv]).is_ok());
    }
}

#[test]
fn icf_creat_error() {
    let args = &["conv=excl,nocreat"];
    assert!(Parser::new().parse(args).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn direct_and_nocache_conflict_on_the_same_file() {
    for operand in ["iflag=direct,nocache", "oflag=direct,nocache"] {
        assert_eq!(
            Parser::new().parse(&[operand]).err(),
            Some(ParseError::MultipleDirectNoCache)
        );
    }

    assert!(
        Parser::new()
            .parse(&["iflag=direct", "oflag=nocache"])
            .is_ok()
    );
}

#[test]
fn parse_icf_token_ibm() {
    let args = &["conv=ibm"];
    let settings = Parser::new().parse(args).unwrap();

    assert_eq!(
        settings.iconv,
        IConvFlags {
            mode: Some(ConversionMode::ConvertOnly(ASCII_TO_IBM)),
            ..Default::default()
        }
    );
}

#[test]
fn parse_icf_tokens_elu() {
    let args = &["conv=ebcdic,lcase"];
    let settings = Parser::new().parse(args).unwrap();
    let ebcdic_lower = get_ctable(Some(Conversion::Ebcdic), Some(Case::Lower)).unwrap();

    assert_eq!(
        settings.iconv,
        IConvFlags {
            mode: Some(ConversionMode::ConvertOnly(ebcdic_lower)),
            ..Default::default()
        }
    );
}

#[test]
fn combined_charset_case_tables_use_gnu_composition_order() {
    for conversion in [Conversion::Ascii, Conversion::Ebcdic, Conversion::Ibm] {
        for case in [Case::Lower, Case::Upper] {
            let actual = get_ctable(Some(conversion), Some(case)).unwrap();
            let case_table = get_ctable(None, Some(case)).unwrap();
            for byte in 0u8..=u8::MAX {
                let expected = match conversion {
                    Conversion::Ascii => case_table[EBCDIC_TO_ASCII[byte as usize] as usize],
                    Conversion::Ebcdic => ASCII_TO_EBCDIC[case_table[byte as usize] as usize],
                    Conversion::Ibm => ASCII_TO_IBM[case_table[byte as usize] as usize],
                };
                assert_eq!(actual[byte as usize], expected);
            }
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn case_tables_honor_single_byte_locale() {
    let Some(lower) = case_table_for_locale(Case::Lower, c"en_US") else {
        return;
    };
    let Some(upper) = case_table_for_locale(Case::Upper, c"en_US") else {
        return;
    };

    assert_eq!(lower[0xc0], 0xe0);
    assert_eq!(upper[0xe0], 0xc0);
}

#[test]
fn parse_icf_tokens_remaining() {
    let args = &[
        "conv=ascii,ucase,block,sparse,swab,sync,noerror,excl,nocreat,notrunc,noerror,fdatasync,fsync",
    ];
    assert_eq!(
        Parser::new().read(args),
        Ok(Parser {
            conv: ConvFlags {
                ascii: true,
                ucase: true,
                block: true,
                sparse: true,
                swab: true,
                sync: true,
                noerror: true,
                excl: true,
                nocreat: true,
                notrunc: true,
                fdatasync: true,
                fsync: true,
                ..Default::default()
            },
            is_conv_specified: true,
            ..Default::default()
        })
    );
}

#[test]
fn parse_iflag_tokens() {
    let args = &["iflag=fullblock,count_bytes,skip_bytes"];
    assert_eq!(
        Parser::new().read(args),
        Ok(Parser {
            iflag: IFlags {
                is_fullblock: true,
                is_count_bytes: true,
                is_skip_bytes: true,
                ..Default::default()
            },
            ..Default::default()
        })
    );
}

#[test]
fn parse_oflag_tokens() {
    let args = &["oflag=append,seek_bytes"];
    assert_eq!(
        Parser::new().read(args),
        Ok(Parser {
            oflag: OFlags {
                is_append: true,
                is_seek_bytes: true,
                ..Default::default()
            },
            ..Default::default()
        })
    );
}

#[test]
fn fullblock_is_invalid_as_an_output_flag() {
    assert_eq!(
        Parser::new().parse(&["oflag=fullblock"]).err(),
        Some(ParseError::OutputFlagNoMatch("fullblock".to_string()))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn parse_iflag_tokens_linux() {
    let args = &["iflag=direct,directory,dsync,sync,nonblock,noatime,noctty,nofollow"];
    assert_eq!(
        Parser::new().read(args),
        Ok(Parser {
            iflag: IFlags {
                is_direct: true,
                is_directory: true,
                is_dsync: true,
                is_sync: true,
                is_nonblock: true,
                is_noatime: true,
                is_noctty: true,
                is_nofollow: true,
                ..Default::default()
            },
            ..Default::default()
        })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn parse_oflag_tokens_linux() {
    let args = &["oflag=direct,directory,dsync,sync,nonblock,noatime,noctty,nofollow"];
    assert_eq!(
        Parser::new().read(args),
        Ok(Parser {
            oflag: OFlags {
                is_direct: true,
                is_directory: true,
                is_dsync: true,
                is_sync: true,
                is_nonblock: true,
                is_noatime: true,
                is_noctty: true,
                is_nofollow: true,
                ..Default::default()
            },
            ..Default::default()
        })
    );
}

// ----- Multiplier Strings etc. -----
macro_rules! test_byte_parser (
    ( $test_name:ident, $bs_str:expr, $bs:expr ) =>
    {
        #[allow(non_snake_case)]
        #[test]
        fn $test_name()
        {
            // let bs_str = String::from($bs_str);
            assert_eq!($bs, parse_bytes_with_opt_multiplier($bs_str).unwrap())
        }
    }
);

test_byte_parser!(test_bytes_n, "765", 765);
test_byte_parser!(test_bytes_c, "13c", 13);

test_byte_parser!(test_bytes_w, "1w", 2);

test_byte_parser!(test_bytes_b, "1b", 512);

test_byte_parser!(test_bytes_k, "1kB", 1000);
test_byte_parser!(test_bytes_K, "1K", 1024);
test_byte_parser!(test_bytes_Ki, "1KiB", 1024);

test_byte_parser!(test_bytes_MB, "2MB", 2 * 1000 * 1000);
test_byte_parser!(test_bytes_M, "2M", 2 * 1024 * 1024);
test_byte_parser!(test_bytes_Mi, "2MiB", 2 * 1024 * 1024);

test_byte_parser!(test_bytes_GB, "3GB", 3 * 1000 * 1000 * 1000);
test_byte_parser!(test_bytes_G, "3G", 3 * 1024 * 1024 * 1024);
test_byte_parser!(test_bytes_Gi, "3GiB", 3 * 1024 * 1024 * 1024);

#[cfg(target_pointer_width = "64")]
#[cfg(test)]
mod test_64bit_arch {
    use super::*;

    test_byte_parser!(test_bytes_TB, "4TB", 4 * 1000 * 1000 * 1000 * 1000);
    test_byte_parser!(test_bytes_T, "4T", 4 * 1024 * 1024 * 1024 * 1024);
    test_byte_parser!(test_bytes_Ti, "4TiB", 4 * 1024 * 1024 * 1024 * 1024);

    test_byte_parser!(test_bytes_PB, "5PB", 5 * 1000 * 1000 * 1000 * 1000 * 1000);
    test_byte_parser!(test_bytes_P, "5P", 5 * 1024 * 1024 * 1024 * 1024 * 1024);
    test_byte_parser!(test_bytes_Pi, "5PiB", 5 * 1024 * 1024 * 1024 * 1024 * 1024);

    test_byte_parser!(
        test_bytes_EB,
        "6EB",
        6 * 1000 * 1000 * 1000 * 1000 * 1000 * 1000
    );
    test_byte_parser!(
        test_bytes_E,
        "6E",
        6 * 1024 * 1024 * 1024 * 1024 * 1024 * 1024
    );
    test_byte_parser!(
        test_bytes_Ei,
        "6EiB",
        6 * 1024 * 1024 * 1024 * 1024 * 1024 * 1024
    );
}

#[test]
#[should_panic]
fn test_neg_panic() {
    let bs_str = format!("{}", -1);

    parse_bytes_with_opt_multiplier(&bs_str).unwrap();
}
