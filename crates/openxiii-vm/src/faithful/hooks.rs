//! The faithful VM installs no improved script hooks.
use crate::call_hooks::{Arguments, CallResult};
use crate::{types::FuncId, value::ObjId, Vm};
use std::ops::ControlFlow;

#[derive(Clone, Copy)]
pub(crate) enum Hook {}

pub(crate) const BEFORE_VERSUS: &[(Hook, &[&str])] = &[];
pub(crate) const BEFORE_COMIC_FRAME: &[(Hook, &[&str])] = &[];
pub(crate) const AFTER_COMIC_FRAME: &[(Hook, &[&str])] = &[];

impl Hook {
    pub(crate) fn run(self, _: &mut Vm, _: FuncId, _: ObjId, _: Arguments) -> ControlFlow<CallResult, Arguments> {
        match self {}
    }
}
