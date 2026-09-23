#[repr(C)]
pub struct ListNode {
    pub value: i32,
    pub next: *mut ListNode,
}

#[no_mangle]
pub unsafe extern "C" fn smallestValue(head: *mut ListNode) -> i32 {
    let mut values = Vec::new();
    let mut current = head;
    while !current.is_null() {
        values.push((*current).value);
        current = (*current).next;
    }
    crate::logic::smallestValue(&values)
}
