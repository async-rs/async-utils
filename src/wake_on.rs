//! Smart pointers to wake tasks on access
use async_std::task::Waker;
use std::ops::{Deref, DerefMut};

/// A wrapper type which wakes tasks whenever the wrapped value is accessed
/// through an `&mut` reference.
///
/// `T` is the type of the value being wrapped. This struct is `Deref` and
/// `DerefMut` for that type, giving `&T` and `&mut T` respectively.
/// When a `Waker` is registered with `set_waker`, that `Waker` is woken
/// whenever the wrapped value is accessed through an `&mut` reference
/// and therefore potentially mutated.
///
/// This is useful when there is a future polling the state of the wrapped
/// value. It needs to be awoken whenever that value changes so that they
/// can check whether or not its value is in a state that will let the
/// future make progress. That future can register the `Waker` from the
/// `Context` it is passed with the `WakeOnWrite` wrapping the value it is
/// interested in so that all mutations cause it to be woken going forward.
///
/// This type isn't effective for observing changes on values with interior
/// mutablity, because it only wakes on `&mut` access.
#[derive(Default, Debug, Clone)]
pub struct WakeOnWrite<T> {
    inner: T,
    waker: Option<Waker>,
}

impl<T> WakeOnWrite<T> {
    /// Create a new `WakeOnWrite` with the given value.
    pub fn new(value: T) -> Self {
        Self {
            inner: value,
            waker: None,
        }
    }

    /// Set the `Waker` to be awoken when this value is mutated.
    ///
    /// Returns the currently registered `Waker`, if there is one.
    pub fn set_waker(wow: &mut Self, waker: Waker) -> Option<Waker> {
        wow.waker.replace(waker)
    }

    /// Removes and returns the currently registered `Waker`, if there is one.
    pub fn take_waker(wow: &mut Self) -> Option<Waker> {
        wow.waker.take()
    }

    /// Returns the currently registered `Waker`, leaving it registered, if
    /// there is one.
    pub fn waker(wow: &Self) -> Option<&Waker> {
        wow.waker.as_ref()
    }
}

impl<T> Deref for WakeOnWrite<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for WakeOnWrite<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.waker.as_ref().map(|w| w.wake_by_ref());
        &mut self.inner
    }
}

#[async_std::test]
async fn wow_wakes_target_on_mut_access() {
    use async_std::future::poll_fn;
    use async_std::prelude::*;
    use async_std::sync::Arc;
    use async_std::sync::Mutex;
    use async_std::task::Poll;
    use pin_utils::pin_mut;
    use std::future::Future;

    let data: Arc<Mutex<WakeOnWrite<u8>>> = Default::default();
    let data_checker = {
        let data_ref = data.clone();
        poll_fn(move |ctx| {
            // This is an inefficient use of futures, but it does work in this
            // case.
            let data_lock_future = data_ref.lock();
            pin_mut!(data_lock_future);
            match data_lock_future.poll(ctx) {
                Poll::Ready(mut lock) => match **lock {
                    10 => Poll::Ready(()),
                    _ => {
                        WakeOnWrite::set_waker(&mut lock, ctx.waker().clone());
                        Poll::Pending
                    }
                },
                Poll::Pending => Poll::Pending,
            }
        })
    };

    let data_incrementor = {
        let data_ref = data.clone();
        async move {
            for _ in 0..10u8 {
                let mut lock = data_ref.lock().await;
                **lock += 1;
            }
        }
    };

    data_checker
        .join(data_incrementor)
        .timeout(core::time::Duration::new(1, 0))
        .await
        .unwrap();
}
