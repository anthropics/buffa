//! Minimal reproducer for rust-lang/rust#128095.

#[cfg(any(feature = "auto_send", feature = "cross_old"))]
pub mod cross;

// ========================================================================
// CASE H: does Cow<'static, str> trigger this? The guide claims "any type
// with lifetime parameters". But Cow is auto-Send (no 'static-bounded
// conditional impl), so if the trigger is the V: 'static bound on
// OwnedView's Send impl, Cow should be fine.
// ========================================================================

#[cfg(feature = "cow_h")]
pub trait ServiceH: Send + Sync + 'static {
    fn handle(
        &self,
        req: std::borrow::Cow<'static, str>,
    ) -> impl std::future::Future<Output = String> + Send;
}

#[cfg(feature = "cow_h")]
struct ImplH;

#[cfg(feature = "cow_h")]
impl ServiceH for ImplH {
    async fn handle(&self, req: std::borrow::Cow<'static, str>) -> String {
        let s: &str = &req;
        some_io().await;
        s.to_string()
    }
}

use std::future::Future;
#[allow(unused)]
use std::marker::PhantomData;

// ── Minimal OwnedView-shaped types ───────────────────────────────────────

// Mirrors buffa-generated view types (e.g. SayRequestView<'a>): plain struct
// with borrowed fields, auto-Send via `&'a str`. An earlier iteration of this
// harness used `PhantomData<*const u8>` + `unsafe impl Send for View<'_>` to
// force a !Send auto-trait; that shape does NOT reproduce E0477 because it
// lacks the crucial `'static` bound on the conditional impl. Getting the mock
// faithful to the real type mattered.
pub struct View<'a> {
    pub name: &'a str,
}

// Two `OwnedView` shapes, toggled by feature:
//
//   default   → mirrors buffa before the fix: manual `unsafe impl` with a
//               `V: 'static` bound. This bound is the trigger for E0477.
//
//   auto_send → mirrors buffa after the fix: no manual impl. Auto-traits
//               give `Owned<V>: Send iff V: Send`, no lifetime bound.
//               The real OwnedView was then `ManuallyDrop<V>` + `Bytes`, both of
//               which forward auto-traits, so a bare `(V,)` is equivalent.

#[cfg(not(feature = "auto_send"))]
pub struct Owned<V>(pub V, PhantomData<*const u8>);
#[cfg(not(feature = "auto_send"))]
unsafe impl<V: Send + 'static> Send for Owned<V> {}
#[cfg(not(feature = "auto_send"))]
unsafe impl<V: Sync + 'static> Sync for Owned<V> {}

#[cfg(feature = "auto_send")]
pub struct Owned<V>(pub V);

pub async fn some_io() {}

// ========================================================================
// CASE A: What we generate today — explicit `fn -> impl Future + Send`
// ========================================================================

pub trait ServiceA: Send + Sync + 'static {
    fn handle(&self, req: Owned<View<'static>>) -> impl Future<Output = String> + Send;
}

struct ImplA;

// This is the failure mode our docs warn about: `async fn` in impl
// against a trait with `+ Send` RPITIT, with a 'static lifetime param
// on the argument. Should hit E0477 per #128095.
#[cfg(feature = "repro_a")]
impl ServiceA for ImplA {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let n = req.0.name;
        some_io().await;
        n.to_string()
    }
}

// The workaround we document — always works.
#[cfg(not(feature = "repro_a"))]
impl ServiceA for ImplA {
    fn handle(&self, req: Owned<View<'static>>) -> impl Future<Output = String> + Send {
        async move {
            let n = req.0.name;
            some_io().await;
            n.to_string()
        }
    }
}

// ========================================================================
// CASE B: Same trait, but defined via trait_variant::make(Send)
// ========================================================================
//
// Key question: does the macro expansion differ from Case A in a way
// that dodges #128095?

#[trait_variant::make(Send)]
pub trait ServiceB: Sync + 'static {
    async fn handle(&self, req: Owned<View<'static>>) -> String;
}

struct ImplB;

// If trait_variant helped, this would compile. If it doesn't, the
// expansion is equivalent to Case A and we gain nothing for the
// multi-threaded (Send-bounded) path.
#[cfg(feature = "repro_b")]
impl ServiceB for ImplB {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let n = req.0.name;
        some_io().await;
        n.to_string()
    }
}

#[cfg(not(feature = "repro_b"))]
impl ServiceB for ImplB {
    fn handle(&self, req: Owned<View<'static>>) -> impl Future<Output = String> + Send {
        async move {
            let n = req.0.name;
            some_io().await;
            n.to_string()
        }
    }
}

// ========================================================================
// CASE C: trait_variant dual-trait form — does the non-Send variant help?
// ========================================================================

#[trait_variant::make(ServiceCSend: Send)]
pub trait ServiceC: 'static {
    async fn handle(&self, req: Owned<View<'static>>) -> String;
}

struct ImplC;

// The non-Send "local" trait has no `+ Send` on the RPITIT, so the
// #128095 desugaring bug shouldn't fire. But this trait is useless
// for tokio multi-threaded dispatch.
#[cfg(feature = "repro_c")]
impl ServiceC for ImplC {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let n = req.0.name;
        some_io().await;
        n.to_string()
    }
}

// Verify: does impl'ing the non-Send local trait get you the Send variant
// via the blanket impl? (This is what our dispatch layer would need.)
#[cfg(feature = "repro_c")]
fn use_as_send_variant<T: ServiceCSend>(_: T) {}
#[cfg(feature = "repro_c")]
fn check_impl_c() {
    use_as_send_variant(ImplC);
}
// ========================================================================
// CASE G: explicit `drop(req)` — workaround claimed in rust#128095
//
// Test against our current trait shape (ServiceA: fn -> impl Future + Send).
// Four variants to isolate what "drop fixes it" actually means.
// ========================================================================

#[cfg(feature = "drop_g1")]
struct ImplG1;
// G1: drop(req) at the end, AFTER using it across an await.
// This is the interesting case — we still get zero-copy access,
// we just add an explicit drop at the tail.
#[cfg(feature = "drop_g1")]
impl ServiceA for ImplG1 {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let n = req.0.name; // borrow into the view
        some_io().await; // req is live across this await
        let out = n.to_string();
        drop(req);
        out
    }
}

#[cfg(feature = "drop_g2")]
struct ImplG2;
// G2: rebind — `let req = req;` at the top of the body.
// The #128095 thread mentions this as an alternative nudge: the rebind
// may shed the fresh-lifetime param from the argument position.
#[cfg(feature = "drop_g2")]
impl ServiceA for ImplG2 {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let req = req;
        let n = req.0.name;
        some_io().await;
        n.to_string()
    }
}

#[cfg(feature = "drop_g3")]
struct ImplG3;
// G3: drop(req) before any await. Already known to work (docs say
// "consume the OwnedView before awaiting"). Verifying for completeness.
#[cfg(feature = "drop_g3")]
impl ServiceA for ImplG3 {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let n = req.0.name.to_string(); // clone out before dropping
        drop(req);
        some_io().await;
        n
    }
}

#[cfg(feature = "drop_g4")]
struct ImplG4;
// G4: baseline — no drop, no rebind. Same body as repro_a, here so
// the four variants diff cleanly against each other.
#[cfg(feature = "drop_g4")]
impl ServiceA for ImplG4 {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let n = req.0.name;
        some_io().await;
        n.to_string()
    }
}

#[cfg(feature = "drop_g5")]
struct ImplG5;
// G5: the guide's "exception" pattern — consume and shadow on line 1.
// Mirrors `let req = req.to_owned_message();`. If E0477 fires at the
// signature level (before body analysis), this should fail too and
// the guide is wrong on this point as well.
#[cfg(feature = "drop_g5")]
impl ServiceA for ImplG5 {
    async fn handle(&self, req: Owned<View<'static>>) -> String {
        let req: String = {
            drop(req);
            String::new()
        }; // consume+shadow, line 1
        some_io().await;
        req
    }
}

#[cfg(feature = "drop_g6")]
struct ImplG6;
// G6: empty body. If this still fails, E0477 is purely signature-level
// and no body workaround can possibly help.
#[cfg(feature = "drop_g6")]
impl ServiceA for ImplG6 {
    async fn handle(&self, _req: Owned<View<'static>>) -> String {
        String::new()
    }
}

#[cfg(feature = "drop_g7")]
struct ImplG7;
// G7: `_` destructure in the signature — don't even bind the argument.
// Some #128095 comments suggest this is the only body-side dodge.
#[cfg(feature = "drop_g7")]
impl ServiceA for ImplG7 {
    async fn handle(&self, _: Owned<View<'static>>) -> String {
        some_io().await;
        String::new()
    }
}
