#![no_std]

use buffa::alloc::{vec, vec::Vec};
use buffa::ProtoList;
use buffa_remote_derive::ProtoList as DeriveProtoList;

#[derive(Clone, PartialEq, Debug, DeriveProtoList)]
#[buffa(remote = Vec<__BuffaIter>)]
struct GenericList<__BuffaIter>(Vec<__BuffaIter>);

impl<__BuffaIter> Default for GenericList<__BuffaIter> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

#[test]
fn from_iter_does_not_shadow_the_element_parameter() {
    let mut list: GenericList<i32> = [1, 2, 3].into_iter().collect();
    assert_eq!(&*list, &[1, 2, 3]);
    list.push(4);
    assert_eq!(&*list, &[1, 2, 3, 4]);
    list.clear();
    assert!(list.is_empty());
}

#[derive(Clone, PartialEq, Debug, DeriveProtoList)]
#[buffa(remote = Vec<r#__BuffaIter>)]
struct RawGenericList<r#__BuffaIter>(Vec<r#__BuffaIter>);

impl<r#__BuffaIter> Default for RawGenericList<r#__BuffaIter> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

#[test]
fn from_iter_handles_raw_element_parameter_names() {
    let list: RawGenericList<i32> = [1, 2, 3].into_iter().collect();
    assert_eq!(&*list, &[1, 2, 3]);
}

mod remote_name {
    use super::*;

    type __BuffaIter<T> = Vec<T>;

    #[derive(Clone, PartialEq, Debug, DeriveProtoList)]
    #[buffa(remote = __BuffaIter<T>)]
    struct NamedList<T> {
        items: __BuffaIter<T>,
    }

    impl<T> Default for NamedList<T> {
        fn default() -> Self {
            Self { items: Vec::new() }
        }
    }

    #[test]
    fn from_iter_does_not_shadow_the_remote_type() {
        let list: NamedList<i32> = [1, 2, 3].into_iter().collect();
        assert_eq!(&*list, &[1, 2, 3]);
    }
}

#[allow(non_upper_case_globals)]
mod const_names {
    use super::*;

    #[derive(Clone, PartialEq, Debug, DeriveProtoList)]
    #[buffa(remote = Vec<T>)]
    struct ConstList<T, const __BuffaIter: usize>(Vec<T>);

    impl<T, const __BuffaIter: usize> Default for ConstList<T, __BuffaIter> {
        fn default() -> Self {
            Self(Vec::new())
        }
    }

    #[test]
    fn from_iter_does_not_shadow_a_const_parameter() {
        let list: ConstList<i32, 4> = [1, 2, 3].into_iter().collect();
        assert_eq!(&*list, &[1, 2, 3]);
    }

    #[derive(Clone, PartialEq, Debug, DeriveProtoList)]
    #[buffa(remote = Vec<__BuffaIter_>)]
    struct SuffixedList<__BuffaIter_, const __BuffaIter: usize, const __BuffaIter__: usize>(
        Vec<__BuffaIter_>,
    );

    impl<__BuffaIter_, const __BuffaIter: usize, const __BuffaIter__: usize> Default
        for SuffixedList<__BuffaIter_, __BuffaIter, __BuffaIter__>
    {
        fn default() -> Self {
            Self(Vec::new())
        }
    }

    #[test]
    fn from_iter_checks_each_suffix_before_using_it() {
        let list: SuffixedList<i32, 4, 8> = [1, 2, 3].into_iter().collect();
        assert_eq!(&*list, &[1, 2, 3]);
    }
}

#[test]
fn from_iter_accepts_empty_and_non_default_elements() {
    #[derive(Clone, PartialEq, Debug)]
    struct Element(i32);

    let empty: GenericList<Element> = core::iter::empty().collect();
    assert!(empty.is_empty());
    let list: GenericList<Element> = vec![Element(1), Element(2)].into_iter().collect();
    assert_eq!(&*list, &[Element(1), Element(2)]);
    assert_eq!(list, GenericList::from(vec![Element(1), Element(2)]));
}
