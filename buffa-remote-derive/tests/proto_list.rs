use buffa::ProtoList;
use buffa_remote_derive::ProtoList as DeriveProtoList;
use smallvec::SmallVec;

#[derive(Clone, PartialEq, Debug, DeriveProtoList)]
#[buffa(remote = smallvec::SmallVec<[T; 4]>)]
struct MyList<T>(pub SmallVec<[T; 4]>);

// Hand-written, not `#[derive(Default)]` — a derived impl would force
// `T: Default`, which `ProtoList<T>` does not require.
impl<T> Default for MyList<T> {
    fn default() -> Self {
        Self(SmallVec::new())
    }
}

#[test]
fn push_and_clear() {
    let mut list = MyList::<i64>::default();
    list.push(1);
    list.push(2);
    list.push(3);
    assert_eq!(&*list, &[1, 2, 3]);
    list.clear();
    assert!(list.is_empty());
}

#[test]
fn from_iter_and_from_vec() {
    let from_iter: MyList<i64> = (1..=3).collect();
    let from_vec = MyList::from(vec![1i64, 2, 3]);
    assert_eq!(from_iter, from_vec);
}

#[test]
fn works_for_non_default_element_type() {
    // `f64` has no `Eq`/`Ord`, exercising that the derive does not require
    // bounds beyond what `ProtoList<T>` itself demands.
    let mut list = MyList::<f64>::default();
    list.push(1.5);
    list.push(2.5);
    assert_eq!(&*list, &[1.5, 2.5]);
}
