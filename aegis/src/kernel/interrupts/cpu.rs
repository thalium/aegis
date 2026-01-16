// CPU interrupts

use x86_64::structures::idt::{InterruptStackFrame, PageFaultErrorCode};

pub extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

pub extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    panic!("page_fault ERROR: {:?}\n{:?}", error_code, stack_frame)
}

// Macro to generate default handlers
#[macro_export]
macro_rules! default_handler {
    ($name:ident) => {
        paste! {
            pub extern "x86-interrupt" fn [<$name _handler>](stack_frame: InterruptStackFrame) {
                panic!(concat!(stringify!($name), " ERROR\n{:#?}"), stack_frame);
            }
        }
    };
}
