#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

#[test]
fn immutable_lock() {
    let arr = [64; 12];
    let mem_lock = Locked::try_new(&arr).unwrap();
    for i in 0..12 {
        assert_eq!(arr[i], mem_lock[i])
    }
}

#[test]
fn mutable_slice_lock() {
    let mut arr = [1, 2, 3, 4];

    {
        let mut mem_lock = LockedMut::try_new(&mut arr[..]).unwrap();

        mem_lock[1] = 42;
        mem_lock[3] = 99;
    }

    assert_eq!(arr, [1, 42, 3, 99]);
}

#[test]
fn zero_sized_type() {
    let value = ();
    let _lock = Locked::try_new(&value).unwrap();
    assert_eq!(std::mem::size_of_val(&value), 0);

    let value2: &[u8] = &[];
    let lock2 = Locked::try_new(value2).unwrap();
    assert_eq!(std::mem::size_of_val(value2), 0);
    assert_eq!(lock2.len(), 0);
}

#[test]
fn immutable_slice_lock() {
    let arr = [1u32, 2, 3, 4, 5];

    let mem_lock = Locked::try_new(&arr[..]).unwrap();

    assert_eq!(&*mem_lock, &[1, 2, 3, 4, 5]);
}

#[test]
fn struct_lock() {
    #[repr(C)]
    struct Data {
        a: u8,
        b: u64,
    }

    let value = Data { a: 10, b: 20 };
    let lock = Locked::try_new(&value).unwrap();
    assert_eq!(lock.a, 10);
    assert_eq!(lock.b, 20);
}

#[test]
fn string_content_vs_string_struct() {
    let value = String::from("hello");

    let content = lock(value.as_str()).unwrap();
    assert_eq!(content.size, 5);

    let string = lock(&value).unwrap();
    assert_eq!(string.size, std::mem::size_of::<String>());
}
