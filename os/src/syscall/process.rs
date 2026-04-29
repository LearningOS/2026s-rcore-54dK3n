//! Process management syscalls
use crate::config::PAGE_SIZE;
use crate::mm::{translated_byte_buffer, MapPermission, PTEFlags, PageTable, VirtAddr};
use crate::task::{
    change_program_brk, current_syscall_count, current_task_mmap, current_task_munmap,
    current_user_token, exit_current_and_run_next, suspend_current_and_run_next,
};
use crate::timer::get_time_us;
use core::{mem::size_of, slice};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

fn copy_bytes_to_user(dst: *mut u8, src: &[u8]) {
    // Copy through translated user pages instead of dereferencing the raw pointer
    // directly, so this still works when the destination crosses a page boundary.
    let buffers = translated_byte_buffer(current_user_token(), dst as *const u8, src.len());
    let mut copied = 0;
    for buffer in buffers {
        let len = buffer.len();
        buffer.copy_from_slice(&src[copied..copied + len]);
        copied += len;
    }
}

fn read_user_byte(src: *const u8) -> Option<u8> {
    let page_table = PageTable::from_token(current_user_token());
    let va = VirtAddr::from(src as usize);
    let pte = page_table.translate(va.floor())?;
    let flags = pte.flags();
    // `trace_read` should fail gracefully on unmapped/kernel/non-readable pages
    // instead of panicking the whole kernel during pointer translation.
    if !pte.is_valid()
        || !flags.contains(PTEFlags::U)
        || !(flags.contains(PTEFlags::R) || flags.contains(PTEFlags::X))
    {
        return None;
    }
    Some(pte.ppn().get_bytes_array()[va.page_offset()])
}

fn write_user_byte(dst: *mut u8, value: u8) -> bool {
    let page_table = PageTable::from_token(current_user_token());
    let va = VirtAddr::from(dst as usize);
    let pte = match page_table.translate(va.floor()) {
        Some(pte) => pte,
        None => return false,
    };
    let flags = pte.flags();
    if !pte.is_valid() || !flags.contains(PTEFlags::U) || !flags.contains(PTEFlags::W) {
        return false;
    }
    pte.ppn().get_bytes_array()[va.page_offset()] = value;
    true
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let time_val_bytes = unsafe {
        slice::from_raw_parts(
            (&time_val as *const TimeVal).cast::<u8>(),
            size_of::<TimeVal>(),
        )
    };
    copy_bytes_to_user(ts.cast::<u8>(), time_val_bytes);
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        0 => read_user_byte(id as *const u8).map_or(-1, |value| value as isize),
        1 => {
            if write_user_byte(id as *mut u8, data as u8) {
                0
            } else {
                -1
            }
        }
        2 => current_syscall_count(id) as isize,
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel: sys_mmap");
    if start % PAGE_SIZE != 0 || port & !0x7 != 0 || port & 0x7 == 0 {
        return -1;
    }
    let end = match start.checked_add(len) {
        Some(end) => end,
        None => return -1,
    };
    if len == 0 {
        return 0;
    }
    // The userspace ABI encodes permissions as RWX in bits [2:0], while
    // MapPermission uses the PTE bit layout, so we translate them explicitly.
    let mut permission = MapPermission::U;
    if port & 0x1 != 0 {
        permission |= MapPermission::R;
    }
    if port & 0x2 != 0 {
        permission |= MapPermission::W;
    }
    if port & 0x4 != 0 {
        permission |= MapPermission::X;
    }
    if current_task_mmap(VirtAddr::from(start), VirtAddr::from(end), permission) {
        0
    } else {
        -1
    }
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    let end = match start.checked_add(len) {
        Some(end) => end,
        None => return -1,
    };
    if len == 0 {
        return 0;
    }
    if current_task_munmap(VirtAddr::from(start), VirtAddr::from(end)) {
        0
    } else {
        -1
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
