use core::arch::asm;
use core::mem::MaybeUninit;

use alloc::string::String;

use spin::Mutex;
use x86_64::instructions::interrupts;

pub use waros_pkg::{
    payload_digests, sha256_hex, verify_bundle_with_embedded_root, VerifyError,
    WARPKG_BOOTSTRAP_KEY_ID, WARPKG_SIGNATURE_SCHEME,
};

use super::manifest::WarPackBundle;

const VERIFY_STACK_SIZE: usize = 256 * 1024;

struct VerifyStack([u8; VERIFY_STACK_SIZE]);

static VERIFY_STACK: Mutex<VerifyStack> = Mutex::new(VerifyStack([0; VERIFY_STACK_SIZE]));

struct VerifyContext<'a> {
    bundle: &'a WarPackBundle,
    result: MaybeUninit<Result<(), VerifyError>>,
}

#[must_use]
pub fn format_trust_root() -> String {
    alloc::format!(
        "{} ({})",
        WARPKG_BOOTSTRAP_KEY_ID,
        WARPKG_SIGNATURE_SCHEME
    )
}

pub fn verify_bootstrap_bundle(bundle: &WarPackBundle) -> Result<(), VerifyError> {
    interrupts::without_interrupts(|| verify_bootstrap_bundle_on_dedicated_stack(bundle))
}

#[inline(never)]
fn verify_bootstrap_bundle_on_dedicated_stack(bundle: &WarPackBundle) -> Result<(), VerifyError> {
    let mut stack = VERIFY_STACK.lock();
    let stack_top = unsafe { stack.0.as_mut_ptr().add(VERIFY_STACK_SIZE) };
    let mut context = VerifyContext {
        bundle,
        result: MaybeUninit::uninit(),
    };

    unsafe {
        run_verify_trampoline((&mut context as *mut VerifyContext<'_>).cast::<u8>(), stack_top);
        context.result.assume_init()
    }
}

#[inline(never)]
extern "C" fn verify_bootstrap_bundle_trampoline(context: *mut u8) {
    let context = unsafe { &mut *context.cast::<VerifyContext<'_>>() };
    context
        .result
        .write(verify_bundle_with_embedded_root(context.bundle));
}

#[inline(never)]
unsafe fn run_verify_trampoline(context: *mut u8, stack_top: *mut u8) {
    unsafe {
        asm!(
            "mov r15, rsp",
            "mov rsp, {stack_top}",
            "and rsp, -16",
            "call {trampoline}",
            "mov rsp, r15",
            stack_top = in(reg) stack_top,
            trampoline = sym verify_bootstrap_bundle_trampoline,
            in("rdi") context,
            lateout("r15") _,
            clobber_abi("C"),
            options(preserves_flags),
        );
    }
}
