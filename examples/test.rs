use std::borrow::Cow;

fn main() {
    // Shared data (initially borrowed)
    let mut data: Cow<str> = Cow::Borrowed("hello world");
    println!("{}", 272460i32.isqrt());
    // Another user tries to modify it
    let mut user1 = data.clone();
    let mut user2 = data.clone();

    // User1 modifies their copy of the data
    if let Cow::Borrowed(s) = &user1 {
        println!("User1 before: {}", s);
    }

    // Now user1 mutates their copy, switching it to Owned
    user1.to_mut().push_str(" from User1");

    // User2 tries to use the data (still Borrowed at this point)
    if let Cow::Borrowed(s) = &user2 {
        println!("User2 sees: {}", s); // Prints "hello world" as user1's change is local to their instance
    }

    // Now user1's modified version:
    if let Cow::Owned(s) = &user1 {
        println!("User1 after mutation: {}", s); // Prints "hello world from User1"
    }
}
