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

// A remote collection with an inherent `extend` that shadows
// `Extend::extend` under plain method-call syntax (inherent methods win in
// method resolution); the derived `push` must call the trait method via a
// fully qualified path. If `push` used plain method-call syntax, the
// inherent `extend` would win resolution and panic here.
#[derive(Clone, PartialEq, Debug)]
struct SneakyVec<T>(Vec<T>);

impl<T> SneakyVec<T> {
    // Never called is the passing state — do not delete as unused.
    #[allow(dead_code)]
    fn extend<I: IntoIterator<Item = T>>(&mut self, _iter: I) {
        panic!("inherent `extend` must not be called by the derived `push`");
    }
}

impl<T> Extend<T> for SneakyVec<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.0.extend(iter);
    }
}

impl<T> AsRef<[T]> for SneakyVec<T> {
    fn as_ref(&self) -> &[T] {
        &self.0
    }
}

impl<T> From<Vec<T>> for SneakyVec<T> {
    fn from(v: Vec<T>) -> Self {
        Self(v)
    }
}

impl<T> FromIterator<T> for SneakyVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(Vec::from_iter(iter))
    }
}

#[derive(Clone, PartialEq, Debug, DeriveProtoList)]
#[buffa(remote = SneakyVec<T>)]
struct MySneakyList<T>(pub SneakyVec<T>);

impl<T> Default for MySneakyList<T> {
    fn default() -> Self {
        Self(SneakyVec(Vec::new()))
    }
}

#[test]
fn push_uses_extend_trait_not_inherent_method() {
    let mut list = MySneakyList::<i64>::default();
    list.push(7);
    assert_eq!(&*list, &[7]);
}

// The struct's own `where`-clause must be carried onto the generated
// `ProtoList` impl alongside the derive's added bounds — this type fails to
// compile (well-formedness of `Self`) if the impl drops `T: Display`.
#[derive(Clone, PartialEq, Debug, DeriveProtoList)]
#[buffa(remote = Vec<T>)]
struct DisplayList<T>(pub Vec<T>)
where
    T: core::fmt::Display;

impl<T: core::fmt::Display> Default for DisplayList<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

#[test]
fn struct_where_clause_is_preserved() {
    let mut list = DisplayList::<i64>::default();
    list.push(5);
    assert_eq!(&*list, &[5]);
}
