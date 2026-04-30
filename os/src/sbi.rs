//! SBI call wrappers

#![allow(unused)]

use crate::config::UART0;
use core::arch::asm;

const SBI_SET_TIMER: usize = 0;
const SBI_SHUTDOWN: usize = 8;
const UART_RBR: usize = UART0;
const UART_THR: usize = UART0;
const UART_LSR: usize = UART0 + 5;
const UART_LSR_DATA_READY: u8 = 1 << 0;
const UART_LSR_THR_EMPTY: u8 = 1 << 5;

/// general sbi call
#[inline(always)]
fn sbi_call(which: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
    let mut ret;
    unsafe {
        asm!(
            "ecall",
            inlateout("x10") arg0 => ret,
            in("x11") arg1,
            in("x12") arg2,
            in("x16") 0,
            in("x17") which,
        );
    }
    ret
}

/// use sbi call to set timer
pub fn set_timer(timer: usize) {
    sbi_call(SBI_SET_TIMER, timer, 0, 0);
}

/// use sbi call to putchar in console (qemu uart handler)
pub fn console_putchar(c: usize) {
    unsafe {
        while (UART_LSR as *const u8).read_volatile() & UART_LSR_THR_EMPTY == 0 {}
        (UART_THR as *mut u8).write_volatile(c as u8);
    }
}

/// use sbi call to getchar from console (qemu uart handler)
pub fn console_getchar() -> usize {
    unsafe {
        if (UART_LSR as *const u8).read_volatile() & UART_LSR_DATA_READY == 0 {
            0
        } else {
            (UART_RBR as *const u8).read_volatile() as usize
        }
    }
}

/// use sbi call to shutdown the kernel
pub fn shutdown() -> ! {
    sbi_call(SBI_SHUTDOWN, 0, 0, 0);
    panic!("It should shutdown!");
}
