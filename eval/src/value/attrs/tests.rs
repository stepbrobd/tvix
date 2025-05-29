use bstr::B;

use super::*;

#[test]
fn test_empty_attrs_iter() {
    let attrs = NixAttrs::construct(0, vec![]).unwrap().unwrap();
    assert!(attrs.iter().next().is_none());
}

#[test]
fn test_map_attrs_iter() {
    let attrs = NixAttrs::construct(1, vec![Value::from("key"), Value::from("value")])
        .expect("simple attr construction should succeed")
        .unwrap();

    let mut iter = attrs.iter().collect::<Vec<_>>().into_iter();
    let (k, v) = iter.next().unwrap();
    assert!(k == &NixString::from("key"));
    assert_eq!(v.to_str().unwrap(), B("value"));
    assert!(iter.next().is_none());
}
