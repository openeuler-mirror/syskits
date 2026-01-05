/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */
use nix::sys::termios::{ControlFlags, InputFlags, LocalFlags, OutputFlags, Termios};

pub trait TermiosFlag: Copy {
    fn is_in(&self, termios: &Termios, group: Option<Self>) -> bool;
    fn apply(&self, termios: &mut Termios, val: bool);
}

impl TermiosFlag for ControlFlags {
    fn is_in(&self, termios: &Termios, group: Option<Self>) -> bool {
        termios.control_flags.contains(*self)
            && group.is_none_or(|g| !termios.control_flags.intersects(g - *self))
    }

    fn apply(&self, termios: &mut Termios, val: bool) {
        termios.control_flags.set(*self, val);
    }
}

impl TermiosFlag for InputFlags {
    fn is_in(&self, termios: &Termios, group: Option<Self>) -> bool {
        termios.input_flags.contains(*self)
            && group.is_none_or(|g| !termios.input_flags.intersects(g - *self))
    }

    fn apply(&self, termios: &mut Termios, val: bool) {
        termios.input_flags.set(*self, val);
    }
}

impl TermiosFlag for OutputFlags {
    fn is_in(&self, termios: &Termios, group: Option<Self>) -> bool {
        termios.output_flags.contains(*self)
            && group.is_none_or(|g| !termios.output_flags.intersects(g - *self))
    }

    fn apply(&self, termios: &mut Termios, val: bool) {
        termios.output_flags.set(*self, val);
    }
}

impl TermiosFlag for LocalFlags {
    fn is_in(&self, termios: &Termios, group: Option<Self>) -> bool {
        termios.local_flags.contains(*self)
            && group.is_none_or(|g| !termios.local_flags.intersects(g - *self))
    }

    fn apply(&self, termios: &mut Termios, val: bool) {
        termios.local_flags.set(*self, val);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_control_flags_is_in() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = ControlFlags::CREAD;
        let group = Some(ControlFlags::CSIZE);

        // Test when flag is not set
        assert!(!flag.is_in(&termios, None));

        // Test when flag is set
        termios.control_flags.insert(flag);
        assert!(flag.is_in(&termios, None));

        // Test with group
        termios.control_flags.remove(flag);
        termios.control_flags.insert(ControlFlags::CS8);
        assert!(!flag.is_in(&termios, group));
    }

    #[test]
    fn test_input_flags_is_in() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = InputFlags::BRKINT;

        // Test when flag is not set
        assert!(!flag.is_in(&termios, None));

        // Test when flag is set
        termios.input_flags.insert(flag);
        assert!(flag.is_in(&termios, None));
    }

    #[test]
    fn test_output_flags_is_in() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = OutputFlags::OPOST;

        // Test when flag is not set
        assert!(!flag.is_in(&termios, None));

        // Test when flag is set
        termios.output_flags.insert(flag);
        assert!(flag.is_in(&termios, None));
    }

    #[test]
    fn test_local_flags_is_in() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = LocalFlags::ECHO;

        // Test when flag is not set
        assert!(!flag.is_in(&termios, None));

        // Test when flag is set
        termios.local_flags.insert(flag);
        assert!(flag.is_in(&termios, None));
    }

    #[test]
    fn test_control_flags_apply() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = ControlFlags::CREAD;

        // Test applying flag
        flag.apply(&mut termios, true);
        assert!(termios.control_flags.contains(flag));

        // Test removing flag
        flag.apply(&mut termios, false);
        assert!(!termios.control_flags.contains(flag));
    }

    #[test]
    fn test_input_flags_apply() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = InputFlags::BRKINT;

        // Test applying flag
        flag.apply(&mut termios, true);
        assert!(termios.input_flags.contains(flag));

        // Test removing flag
        flag.apply(&mut termios, false);
        assert!(!termios.input_flags.contains(flag));
    }

    #[test]
    fn test_output_flags_apply() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = OutputFlags::OPOST;

        // Test applying flag
        flag.apply(&mut termios, true);
        assert!(termios.output_flags.contains(flag));

        // Test removing flag
        flag.apply(&mut termios, false);
        assert!(!termios.output_flags.contains(flag));
    }

    #[test]
    fn test_local_flags_apply() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let flag = LocalFlags::ECHO;

        // Test applying flag
        flag.apply(&mut termios, true);
        assert!(termios.local_flags.contains(flag));

        // Test removing flag
        flag.apply(&mut termios, false);
        assert!(!termios.local_flags.contains(flag));
    }
}
