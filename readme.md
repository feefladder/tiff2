# Tiff2 crate

Similar in function and planned lifespan as arrow2 crate:
- Async support
- seprarate IO- and CPU-intensive work
- delegates parallelism downstream
- optionally delegates io downstream

Now one very big hrudle to overcome is the fact that we only want to fetch (and decode) the relevant parts of our tiff and not the irrelevant parts. That means that we will need to be able to deal with IFDs that are partially loaded. e.g:
```rust
pub enum IfdEntry {
    Offset(u64),
    Value(Value),
}

pub struct Ifd {
  sub_ifds: Vec<Ifd>,
  data: BTreeMap<Tag, IfdEntry>
}

impl ifd {
    /// Get a tag. Will return None if the tag isn't present (in this tiff/Image)
    fn get_tag(&self, tag: Tag) -> Option<IfdEntry>;
    /// Get a tag, reading in the data when needed
    fn retrieve_tag<R>(&mut self, tag: Tag, reader: R) -> Result<Option<Value>, TiffError>;
    /// Retrieve a tag, returning error if it doesn't exist
    fn retrieve_required_tag<R>(&mut self, tag: Tag, reader: R) -> Result<Value, TiffError>
}
```
But then also, we'd want to be able to "know" whether a datastructure contains all relevant metadata - if we'd want to spawn many readers that use the same `Arc<Image>`, we'd be unable to change that `Image`. In that case, the following would make more sense:
```
pub struct Ifd {
    sub_ifds: Vec<Ifd>,
    pending_data: Vec<(Tag, Pin<Box<dyn Future>>)>
    data: BTreeMap<Tag, IfdEntry>
}
```

## API

This crate is not meant for reading tiff files, but rather for building more specialized tiff readers on top of. However, a rudimentary tiff reader is still implemented to show how that would work.

The following use-cases were taken as example in the design:
1. Reading a specific tile at a given overview level as quickly as possible
2. Mapping application: Reading overlapping tiles of a bbox at given overview level
3. bevy_terrain: acting as a tile server over multiple overview levels

```
|Ifd1|Ifd2|Ifd3|-Ifd1TagData-|-Ifd2TagData-|-Ifd3TagData-|--Image1Data--|--Image2Data--|--Image3Data--|
   \--->points to--->/\---------------->points to-------------->/
```

These are rather different, especially in how eagerly they all should read in
the tag data. For 1, we'd want to be done with 3 requests, while not loading the
complete tag data. For 2 we'd want to
eagerly load a single overview, and for 3 we'd want to eagerly load in most of
the tiff's metadata. Now, that's the case for COGs. Other tiffs may have
different layouts with differing use-cases.

### todo:

- find a better name for `CogReader` trait
- harmonize `Value` between "encoder" and decoder. Options:
  - `Value` (possibly recursive) enum:
    - Nice when there is only a single value
    - Difficult to determine type based on List type
    - IFD is still an offset
  - `BufferedValue`: stored as bytes sequence <- I like actually
    - need to do special indexing strats
    - nice for reading/writing "little-copy"
    - not re-organizing data more than needed just "feels nice"
    - IFD should intuitively store the bytes of the IFD <- contrary to current
      impl, breaks recursion
  - `ProcessedValue`: stored as Vec<Value> <- should not be used together with `Value::List`
    - Why is this any different than recursing Value?
  - Logical impl: Read: BufferedValue -> Value Write: Value -> BufferedValue
    - actually eleganter solution would be to use bytemuck and BufferedValue
