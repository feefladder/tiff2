# Tiff2 crate

Similar in function and planned lifespan as arrow2 crate:
- Async support
- seprarate IO- and CPU-intensive work
- delegates parallelism downstream
- delegates io downstream using a trait
- primary decoder impl is geared towards COGs, but doesn't have the geo stuff
- similar in structure to `image-tiff`, so code can be copied over easily

Now one very big hurdle to overcome is the fact that we only want to fetch (and decode) the relevant parts of our tiff and not the irrelevant parts. That means that we will need to be able to deal with IFDs that are partially loaded. e.g:
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
1. Reading a specific set of tiles at a given overview level as quickly as possible
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

### Organization

The crate is split up in three parts:

1. Data structures
2. Decoders
3. Encoders (todo/no focus)

Data structures are shared between decoders and encoders. All three have a
further hierarchial structure:

1. Entry (tag data)
2. Ifd (generic)
3. ChunkOpts: All relevant options for _independently_ encoding/decoding an
   image chunk
4. Image/ImageMeta: 
   ```rust
   {
     ifd: Ifd,
     opts: Arc<ChunkOpts>, // immutable since we should decide on those before starting the encoding/decoding process.
     chunk_offsets: BufferedEntry, //mutable, since it could be partial or whatevs
     chunk_bytes: BufferedEntry,
   }
   ```
4. Tiff: structures multiple images with metadata. To be re-implemented for
   COG/OME if needed

Decoder:

1. 

#### Statefullness in the decoder/encoder


Decoding a tiff has multiple steps:
1. Reading+decoding Image ifd(s) (&mut self)
2. Reading+decoding relevant tag data (&mut self)
3. reading+decoding chunks (&self -> Future<Output=DecodingResult>)

There are some crates that implement similar mechanics:
1. [image-png]() uses a global (per-file) state
2. [parquet]() 
3. [users answers](https://users.rust-lang.org/t/cloning-a-reader-non-idiomatic/119794/4): 
   > For some of the container formats I've handled, images included, I've found it convenient to split an initial metadata parse that gives you a read-only, shared source, from readers that share said source - though there's still a lot of room to play in that area.

The pictured use-case is a mapping application, where a user freely moves around
the map, 
1. panning (needing more chunks from the same overview/image)
2. zooming in/out (needing chunks from a different overview/image).
esp. 2 gives problems: For reading in a new IFD, we would have to change state (to 1), read+decode the ifd, then (2.) read+decode its tag data and only then can we read in chunks at the new zoom level. The question is how to get an `&mut self` when we could be decoding chunks at the same time? Or use internal mutability - locking hell? 

Adding another overview to the source makes it no longer read-only. Data required for tile retrieval and decoding, however, is rather small and doesn't change. Thus a `Vec<Arc<Image>>` would be enough to read all images contained in that vec. Problems arise when we want to add another `Arc<Image>` to the vec, or want internal mutability.

That is:
```rust
struct CogDecoder {
  /// OverviewLevel->Image map (could be a vec)
  images: HashMap<OverviewLevel, Arc<tiff2::Image>>,
  geo_data: Idk,
  reader: Arc<impl CogReader>
}

impl CogDecoder {
  /// requiring mutable access to self is suboptimal
  /// actually solved
  fn get_chunk(&mut self, i_chunk: u64, zoom_level: OverviewLevel) -> TiffResult<impl Future<Output = DecodingResult>/* + Send */> {
    match self.images.get(zoom_level) {
      // this will make the caller 
      None => Err(TiffError::ImageNotLoaded(zoom_level)), // in this piece of code, we'd have to await IFD retrieval+decoding
      Some(img) => Ok(img.clone().decode_chunk(i_chunk)) // since this returns a future that doesn't reference self, we are happy
    }
  }
}

impl Image {
  // better move this to decoder, only make image return the offset and length
  fn decode_chunk<R>(&self, reader: R, i_chunk: u64) -> impl Future<Output = DecodingResult>{
    let chunk_offset = self.chunk_offsets[i_chunk];
    let chunk_bytes = self.chunk_bytes[i_chunk];
    let chunk_opts = self.chunk_opts.clone();
    async move {
      // don't mention `self` in here, see [stackoverflow](https://stackoverflow.com/a/77845970/14681457)
      ChunkDecoder::decode(reader, chunk_offset, chunk_bytes, chunk_opts)
    }
  }
}

#[tokio::test]
fn test_concurrency() {
  let decoder = CogDecoder::from_url("https://enourmous-cog.com").await.expect("Decoder should build");
  decoder.read_overviews(vec![0,5]).await.expect("Decoder should read ifds");
  // get a chunk from the highest resolution image
  let chunk_1 = decoder.get_chunk(42, 0);
  // get a chunk from a lower resolution image
  let chunk_2 = decoder.get_chunk(42, 5);
  let data = (chunk_1.await, chunk_2.await);
}

#[tokio::test]
fn test_concurrency_fail() {
  let decoder = CogDecoder::from_url("https://enourmous-cog.com").await.expect("Decoder should build");
  decoder.read_overviews(vec![0]).await.expect("decoder should read ifds");
  // get a chunk from the highest resolution image
  let chunk_1 = decoder.get_chunk(42, 0);
  // get a chunk from a lower resolution image
  let chunk_2 = decoder.get_chunk(42, 5); //panic!
  let data = (chunk_1.await, chunk_2.await);
}

// how HeroicKatana would do it if I understand correctly:
#[tokio::test]
fn test_concurrency_recover() {
  let decoder = CogDecoder::from_url("https://enourmous-cog.com").await.expect("Decoder should build");
  decoder.read_overviews(vec![0]).await.expect("decoder should read ifds");
  // get a chunk from the highest resolution image
  let chunk_1 = decoder.get_chunk(42, 0).unwrap();
  // get a chunk from a lower resolution image
  if let OverviewNotLoadedError(chunk_err) = decoder.get_chunk(42, 5).unwrap_err() {
    // read_overviews changes state of the decoder to LoadingIfds
    decoder.read_overviews(chunk_err).await;
  }
  let chunk_2 = decoder.get_chunk(42,5);
  let data = (chunk_1.await, chunk_2.await);
}

#[tokio::test]
fn test_concurrency_recover_problem() {
  let decoder = CogDecoder::from_url("https://enourmous-cog.com").await.expect("Decoder should build");
  decoder.read_overviews(vec![0]).await.expect("decoder should read ifds");
  // get a chunk from the highest resolution image
  let chunk_1 = decoder.get_chunk(42, 0).unwrap();
  // get a chunk from a lower resolution image
  if let OverviewNotLoadedError(chunk_err) = decoder.get_chunk(42, 5).unwrap_err() {
    // read_overviews changes state of the decoder to LoadingIfds
    decoder.read_overviews(chunk_err); // no await
  }
  let chunk_2 = decoder.get_chunk(42,5);
  let data = (chunk_1.await, chunk_2.await);
}
```
The last problem, with statefullness would be solved approx like:
```rust
struct CogDecoder {
  /// OverviewLevel->Image map (could be a vec)
  images: HashMap<OverviewLevel, tiff2::Image>,
  /// Ifds should all be in the first chunk, so we can load them
  ifds: Vec<Ifd>
  byte_order: ByteOrder,
  geo_data: Idk,
  reader: Arc<impl CogReader>,
}

impl CogDecoder {
  async fn read_overviews(&mut self, levels: Vec<OverviewLevel>) {
    // there are only further states, from which we can always return
    self.change_state(DecoderState::LoadingTagData).await;
    levels
      .filter(|level| !self.images.contains_key(level))
      .map(|l| (l, Image::check_ifd(self.ifds[l]))
      .map(|(l, req_tags)| (l, self.reader.read_tags(req_tags)))
      .collect::<TiffError<_>, _>()?;
    for l in levels {
      let req_tags
    }
    self
  }
  async fn get_chunk(&self, i_chunk: u64, zoom_level: OverviewLevel) -> TiffResult<DecodingResult> {
    match self.state {
      // is there some magic that we can await state changes in ourselves?
      DecoderState::Ready => {},
      DecoderState::LoadingIfds => return TiffError::WrongState(),
      _ => return TiffError::WrongState(),
    }
    match self.images.get(zoom_level) {
      None => TiffError::OverviewNotLoadedError(zoom_level), // in this piece of code, we'd have to await IFD retrieval+decoding
      Some(img) => img.decode_chunk(i_chunk) // since this returns a future that doesn't reference self, we are happy
    }
  }
}
```
#### Data structures

```rust
pub struct BufferedEntry {
  tag_type: TagType,
  count: usize,
  data: Vec<u8>,
}
```
The core struct is an IFD.  
An IFD can hold sub-IFDs.
Therefore, it looks like:
```rust
pub struct Ifd {
   sub_ifds: Vec<Ifd>,
   data: BTreeMap<Tag, BufferedEntry>
}
```
A more specialized version is an Image.  
```rust
pub struct Image {
    ifd: Ifd,
    chunk_opts: Arc<ChunkOpts>,
    chunk_offsets: BufferedEntry,
    chunk_bytes: BufferedEntry,
}
```

### Notable changes with image-tiff:

- use of BufferedEntry in stead of Value everywhere
- Ifd and other building blocks have a more central place
- ChunkOpts is taking some place of Image
- 

### todo:

- find a better name for `CogReader` trait
- harmonize `Value` between "encoder" and decoder. Options:
  - `Value` (possibly recursive) enum:
    - Nice when there is only a single value
    - Difficult to determine type based on List type
    - IFD is still an offset
  - **`BufferedValue`: stored as bytes sequence <- I like actually**
    - need to do special indexing strats
    - nice for reading/writing "little-copy"
    - not re-organizing data more than needed just "feels nice"
    - IFD should intuitively store the bytes of the IFD <- contrary to current
      impl, breaks recursion
  - `ProcessedValue`: stored as Vec<Value> <- should not be used together with `Value::List`
    - Why is this any different than recursing Value?
  - Logical impl: Read: BufferedValue -> Value Write: Value -> BufferedValue
    - actually eleganter solution would be to use bytemuck and BufferedValue

