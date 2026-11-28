//! Interface and menu code: C++ code of GUI.dll and Engine.dll (UInteraction*,
//! UConsole) with no line-by-line equivalent to port, because the VM provides it another way:
//! StaticClass, operator new, InternalConstructor, constructors and destructors, operator=,
//! MyStaticSize (the VM's object system); event* and delegate* (ProcessEvent: Vm::event,
//! gui_event, gui_delegate); Catch@ handlers and unnamed functions (destructors of FString and
//! of TArray, "deleting destructors", __ArrayUnwind, _ftol, destructors of static classes at
//! exit: memory and exceptions handled by Rust). Functions that are useless in single-player on PC are
//! not reimplemented.

