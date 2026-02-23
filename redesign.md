# Redesign ramble

So yeah, there's this async-tiff crate right now, but development diverged a bit
and so I'm thinking of starting development here again.  I also did the entire outward-facing api a bit as an afterthought, so here's some thought on that:

## Limiting requests: Caching

The thing is using cogs
directly from Zenodo and they have a 133 requests/min hard rate limit. So the
objective is to get a guaranteed number of requests for metadata fetching from cogs.

That's easy: 2. (for cogs)

ifd decoding has 3 steps:

1. get the number of ifd entries
2. read all ifd entries and values that fit in the offset field
3. read all out-of-offset values and add to data
4. if you have EXIF tags or other infinitely flexible stuff, there may be more, but I don't care

Also, the ifd is guaranteed to follow 1, so any cacher can just fetch some (4-ish) kB after 1 and 2. will always be cache hits. 3. can be large, especially in large COGs. For example, a 30m map of africa covers 30_370_000 km². with a tile size of 256x256 [GDAL default](https://gdal.org/en/stable/drivers/raster/cog.html#reprojection-related-creation-options) has like 6 MB of this metadata, which is too big for a default prefetch in my opinion.

Like in this screencast, you can see an observable latency difference between big and small tiffs:

[![asciicast](https://asciinema.org/a/SEPthxKSMkQ0Eoa06jAtbgdEI.svg)](https://asciinema.org/a/SEPthxKSMkQ0Eoa06jAtbgdEI)

that's because small tiffs only fetch 16kB while big tiffs fetch ~6MB. The only real requirement from the reader (for either exponential prefetch or anything else) is that out-of-offset values are requested as single chunks.

depending on some pesky implementation details, exponential prefetch will take 3 requests, while:

```rust
pub trait MetaReader {
    /// generic read function should not do any caching normally
    fn read(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>>;
    /// read the number of entries in this ifd
    /// 
    /// (first request of an ifd, so you may prefetch like a kB as IFD size estimate)
    fn read_n(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        self.read(range)
    }
    /// read a value that didn't fit in the offset fields
    fn read_value(&mut self, tags: Range<u64>) -> BoxFuture<Result<Bytes>> {
        self.read(range)
    }
    /// clear the cache; we're done reading metadata
    fn clear(&mut self) -> Result<Bytes>;
}

pub trait MetaFetch {
    fn fetch(&self, range: Range<u64>) -> BoxFuture<Result<Bytes>>;
}

impl<T: MetaFetch> MetaReader for T {
    fn read(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        self.fetch(range)
    }
}

struct CogReader {
    fetch: Arc<dyn MetaFetch>,
    cache: Vec<u8>, // or Bytes
}

impl CogReader {
    async fn new(reader: Arc<dyn MetaReader>, prefetch: usize) -> Result<Self> {
        let cache = reader.read(0..prefetch)?;
    }
}

impl MetaReader for CogReader {
    fn read(&self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        async move {
            if range.end <= self.cache.len() {
                Ok(Bytes::copy_from_slice(self.cache.slice(range.start as usize..range.end as usize)))
            } else {
                self.fetch.fetch(range).await
            }
        }.boxed()
    }

    fn read_value(&self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        async move {
            if range.end < self.cache.len() {
                Ok(Bytes::copy_from_slice(self.cache.slice(range.start as usize..range.end as usize)))
            } else {
                let range_len = range.end-range.start;
                // upper bound on metadata based on bigtiff and in-order reading of values
                // also works otherwise, but then it's not that efficient
                let ubound = range_len*2+4*range_len.sqrt();
                // there's a thingy that if we're accidentally reading a normal tiff we'd sometimes read past the end of the file, so here's a small heuristic to test if we're at the start of the file.
                // plz list invariants and stuff
                // so, we SHOULD be doing some sort of exponential prefetch
                // that means that what we fetch extra should be at the start of the file
                // so, the range to fetch is `range.start..range.start + ubound`
                // and this should somehow be in an exponential prefetch
                // maybe check if range.start-ubound < cache_len
                // but in some cases range.start-ubound will overflow so we need a saturating_sub
                // and casting shenanigans
                if range.start.saturating_sub(ubound) < cache_len {
                    let cache = self.reader.read(cache_len..range.start + ubound).await?;
                    let w = self.cache.write().await?
                    w.append(res[..]);
                    self.cache_
                    Ok(w.slice(range.start as usize..range.end as usize))
                } else {
                    // just get the data
                    self.reader.read(range).await
                }
            }
        }
    }
}

// reader that doesn't know shit and only really prefetches ifds
pub struct GenericReader {
    fetch: Arc<dyn MetaFetch>,
    cache: Bytes,
    cache_range: Range<u64>,
    block_size: usize,
}

impl MetaReader for GenericReader {
    fn read(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        if range.start >= self.cache_range.start && range.end <= self.cache_range.start {
            let res = Bytes::copy_from_slice(self.cache.slice(/*magic*/))
        }
        self.fetch.fetch(range)
    }

    fn read_n(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        // swap/make the IFD cache
        async move {
            let c_range = range.start as usize..range.start as usize + self.block_size
            let res = self.fetch.fetch(c_range).await?;
            self.cache = res;
            self.cache_range = c_range;
            Ok(self.cache[..(range.end-range.start) as usize])
        }
    }
}
```
Ok, so I had a bit of a ramble with an llm and thought this structure could be made flexible:
```
+----------+
|MetaParser|  synchronous, stateless, parses stuff based on passed-in ranges
+----------+
      |
      |
     \/
/---------\
|MetaReader| async-sync interface determines whether to read IFDs concurrently or not
\---------/
     |
     |
     \/
 /-------\
|MetaCache| async, structure-aware caching layer
 \-------/
     |
    \/
(MetaFetch) async, just talks to the network layer
```
NOTE: at this point, I've swapped some names

and then users would mainly:
- interop with the `MetaReader`
- implement their `MetaFetch`
So `MetaReader` could look like:
```rust
pub trait MetaReader {
    /// open a tiff file, parsing the header
    fn open(cache: impl MetaCache) -> BoxFuture<Result<Self>>;
    /// read all ifds
    fn read_all_ifds(&mut self) -> BoxFuture<Result<TiffMetaData>>;
    /// read the next ifd
    fn read_next_ifd(&mut self) -> BoxFuture<Result<()>>;
    /// whether we are bigtiff
    fn bigtiff(&self) -> bool;
    /// byte order
    fn endianness(&self) -> Endianness;
    /// some other getters that we'd have control over if there was only one of us
    fn ifd(&self, idx: usize) -> &Ifd;
    fn ifds(self) -> &[Ifd]
}

pub struct SequentialMetaReader {
    ifds: Vec<Ifd>
    cache: Box<dyn MetaCache>
    next_ifd: u64,
    bigtiff: bool,
    endianness: Endianness,
}

impl MetaReader for SequentialMetaReader {
    fn open(cache: impl MetaCache) -> BoxFuture<Result<Self>> {
        async move {
            let (bigtiff, endianness, next_ifd) MetaParser::header(cache.read(0..??).await?[..])? {
                Self {
                    ifds: Vec::new(),
                    cache,
                    next_ifd,
                    bigtiff,
                    endianness,
            }
        }
    }

    fn read_all_ifds(&mut self) -> BoxFuture<Result<TiffMetadata>> {
        while self.next_ifd != 0 {
            self.read_next_ifd()?
        }
        Ok(TiffMetadata::From(self.ifds))
    }

    fn read_next_ifd(&mut self) -> BoxFuture<Result<&Ifd>> {
        async move {
            let n_entries = self.cache.read_n(self.next_ifd).await?;
            let (mut fit_entries: BTreeMap<Tag, Entry>, todo: BTreeMap<Tag, Offset>, next_ifd) = MetaParser::ifd(bigtiff, &cache.read(self.next_ifd+bla..self.next_ifd+blabla));
            self.next_ifd = next_ifd;
            for (tag, offset) in todo {
                let entry = Entry::new(offset, self.cache.read_value(offset.range()).await?);
                fit_entries.insert(tag, entry)
            }
            self.ifds.insert(Ifd::from(fit_entries)?);
            Ok(())
        }
    }
}
```
## Concurrent IFD reads?

Since the previous reader was called `SequentialReader` I had the genious idea of making the reader concurrent
```rust
pub struct ConcurrentMetaReader {
    ifds: Vec<Ifd>,
    cache: Box<dyn MetaCache>,
    next_ifd: u64,
    big: bool,
    endianness: Endianness,
}

impl MetaReader for ConcurrentMetaReader {
    fn open(cache: MetaCache) -> BoxFuture<Result<Self>> {
        async move {
            let (endianness, big, next_ifd) = MetaParser::header(&cache.read(..??).await?);
            Ok(Self {
                ifds: Vec::new(),
                cache,
                next_ifd,
                big,
                endianness,
            })
        }
    }

    fn read_all_ifds(&mut self) -> BoxFuture<Result<Self>> {
        async move {
            let todo = VecDeque::new();
            todo::push(self.next_ifd);

            
            let n_entries = self.cache.read_n(self.next_ifd);
            let buf = cache.read(self.next_ifd+bla..self.next_ifd+blabla).await?;
            let next_ifd = MetaParser::next(self.big, n_entries, &buf);
            // SO NOW WE CAN ALREADY START BOTH READING NEXT AND PARSING CURRENT
            // BUT HOW???
        }
    }
}
```
that concurrency would need some sort of a state machine thing, where we could do pipelining:
```
            io                                    io
|ifd_offset|->|ifd_buffer,next|->|parsed_ifd,todo|->|parsed_ifd|
                           |         io                                    io
                         |ifd_offset|->|ifd_buffer,next|->|parsed_ifd,todo|->|parsed_ifd|
                                                    |         io                                    io
                                                  |ifd_offset|->|ifd_buffer,next|->|parsed_ifd,todo|->|parsed_ifd|
                                                                             |         io                                    io
     io            io                                                      |ifd_offset|->|ifd_buffer,next|->|parsed_ifd,todo|->|parsed_ifd|
|0_0|->|0_1|->|0_2|->|0_3|
         |  io            io
       |1_0|->|1_1|->|1_2|->|1_3|
                |  io            io
              |2_0|->|2_1|->|2_2|->|2_3|
which gets interleaved:
|0_0|->|0_1|->|1_0|->|0_2|->|1_1|->|2_0|->|0_3|->|1_2|->|2_1|->|1_3|->|2_2|->|2_3|
     io           |----io---|          |---------io-----|
                         |----------io----|          |----io---|
```
and in code:
```rust

enum IfdState {
    First(Box<dyn MetaCache>, u64, bool),
    Second(Box<dyn MetaCache>, Bytes, bool, u64),
    Third(Box<dyn MetaCache>, BTreeMap<Tag, Entry>, BTreeMap<Tag, Offset>),
    Done(BTreeMap<Tag, Entry>)
}

enum IfdPoll {
    Pending,
    NextKnown(u64),
    Done(BTreeMap<Tag, Value>)
}

impl IfdState {
    async fn advance(&mut self) -> Result<IfdPoll> {
        match self {
            Self::First(cache, ifd_loc, big) => {
                let n_entries: u64 = cache.read_n(ifd_loc, big).await?
                let buf = cache.read(ifd_loc..ifd_loc+n_entries*entry_size).await?
                let next_offset = MetaParser::next(buf, big, n_entries);
                // magic to make self IfdState::Second
                Ok(IfdPoll::NextKnown)
            }
            Self::Second(cache, buf, bigtiff, n_entries) {
                let (fit_entries, todo) = MetaParser::ifd(buf, bigtiff, n_entries);
                // magic to make self IfdState::Third
                Ok(IfdPoll::Pending)
            }
            Self::Third(cache, fit_entries, todo) {
                cache.read_values(todo).await? // yeah not compiling i don't care

            }
        }
    }
}

```
the conclusion at this point is that this pipelining is totally way too complex
for a maximum of two concurrent requests. But at least now I have a better
understanding of `async`.

So is at this point the entire `MetaReader` interface still needed? I think not...   concurrency is overrated.
The only easy concurrency gain would be from a `coalesce_ranges` and a `read_values` function to get out-of-bounds concurrently, like:
```rust
pub trait MetaCache {
    /// generic read function should not do any caching normally
    fn read(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>>;
    /// read the number of entries in this ifd
    /// 
    /// (first request of an ifd, so you may prefetch like a kB as IFD size estimate)
    fn read_n(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        self.read(range)
    }
    /// read a value that didn't fit in the offset fields
    /// 
    /// The default implementation falls back to `read`
    fn read_value(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        self.read(range)
    }
    /// read multiple values that didn't fit in the offset fields
    /// 
    /// the default implementation calls `read_value` sequentially
    fn read_values(&mut self, ranges: &[Range<u64>]) -> BoxFuture<Result<Bytes>> {
        async move {
            let res = Vec::with_capacity(ranges.len())
            for range in ranges {
                res.push(self.read_value(range)?);
            }
            res
        }.boxed()
    }

    /// clear the cache; we're done reading metadata
    fn clear(&mut self) -> Result<Bytes>;
}


struct CogCache {
    fetch: Arc<dyn MetaFetch>,
    cache: Vec<u8>, // or Bytes
}

impl CogCache {
    async fn new(reader: Arc<dyn MetaReader>, prefetch: usize) -> Result<Self> {
        let cache = reader.read(0..prefetch)?;
    }
}

impl MetaCache for CogCache {
    fn read(&self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        async move {
            if range.end <= self.cache.len() {
                Ok(Bytes::copy_from_slice(self.cache.slice(range.start as usize..range.end as usize)))
            } else {
                self.fetch.fetch(range).await
            }
        }.boxed()
    }

    fn read_value(&self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        async move {
            if range.end < self.cache.len() {
                Ok(Bytes::copy_from_slice(self.cache.slice(range.start as usize..range.end as usize)))
            } else {
                let range_len = range.end-range.start;
                // upper bound on metadata based on bigtiff and in-order reading of values
                // also works otherwise, but then it's not that efficient
                let ubound = range_len*2+4*range_len.sqrt();
                // there's a thingy that if we're accidentally reading a normal tiff we'd sometimes read past the end of the file, so here's a small heuristic to test if we're at the start of the file.
                // plz list invariants and stuff
                // so, we SHOULD be doing some sort of exponential prefetch
                // that means that what we fetch extra should be at the start of the file
                // so, the range to fetch is `range.start..range.start + ubound`
                // and this should somehow be in an exponential prefetch
                // maybe check if range.start-ubound < cache_len
                // but in some cases range.start-ubound will overflow so we need a saturating_sub
                // and casting shenanigans
                if range.start.saturating_sub(ubound) < cache_len {
                    let cache = self.reader.read(cache_len..range.start + ubound).await?;
                    let w = self.cache.write().await?
                    w.append(res[..]);
                    self.cache_
                    Ok(w.slice(range.start as usize..range.end as usize))
                } else {
                    // just get the data
                    self.reader.read(range).await
                }
            }
        }
    }
}

// reader that doesn't know shit and only really prefetches ifds
pub struct GenericCache {
    fetch: Arc<dyn MetaFetch>,
    cache: Bytes,
    cache_range: Range<u64>,
    block_size: usize,
}

impl MetaCache for GenericReader {
    fn read(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        if range.start >= self.cache_range.start && range.end <= self.cache_range.start {
            let res = Bytes::copy_from_slice(self.cache.slice(/*magic*/))
        }
        self.fetch.fetch(range)
    }

    fn read_n(&mut self, range: Range<u64>) -> BoxFuture<Result<Bytes>> {
        // swap/make the IFD cache
        async move {
            let c_range = range.start as usize..range.start as usize + self.block_size
            let res = self.fetch.fetch(c_range).await?;
            self.cache = res;
            self.cache_range = c_range;
            Ok(self.cache[..(range.end-range.start) as usize])
        }
    }

    fn read_values(&mut self, ranges: &[Range<u64>]) -> BoxFuture<Result<Vec<Bytes>>> {
        coalesce_ranges(
            ranges,
            |range| self.fetch.fetch(range),
            OBJECT_STORE_COALESCE_DEFAULT,   // or should this be done a layer deeper?
        )
    }
}
```

## Tag Registry?

Yeah, so this is mainly the real reason I'm writing this now, is that there was
this tag registry idea and I liked it and I wanted efficient caching
(implementing at two levels is really really ugly, sorry) so then I came here, but I
_do_ really like this tag registry. The main question is: Where to call for the
parsing of custom tags?

I think when we create the `Ifd` from a `BTreeMap<Tag, Value>`, that's also what
`async-tiff` does. And ideally we'd be a drop-in replacement, because otherwise
there's really no hope of ever making it into `geotiff`

```rust
pub trait CustomTagParser: Any {
    fn tags(&self) -> &[Tag]
    fn parse(&self, Tag, Value) -> BoxFuture<Result<()>>;
}
// and there was some magic with a blanket trait to make (deep) cloning possible while keeping it object-safe

// yeah, we use arc so one parser can be shared between multiple tags
pub struct CustomTagRegistry(BTreeMap<Tag, Arc<dyn CustomTagParser>>)

impl CustomTagRegistry {
    fn register(parser: Arc<dyn CustomTagParser>) -> Result<()> {
        let tags = parser.tags();
        if <overlapping tags> return Err();
        for tag in tags {
            self.0.insert(tag, parser.clone())
        }
        Ok()
    }

    fn parse(tag: Tag, value: Value) -> BoxFuture<Result<()>> {
        if self.0.contains_key(tag) {
            self.0[tag].parse(tag, value)
        } else {
            async{Ok(())}.boxed()
        }
    }

    fn get(tag) -> Option<Arc<dyn CustomTagParser>> {
        self.0.get(tag)
    }
}
```

And then that'd be added in

```rust
impl Ifd {
    async fn from(tags: BTreeMap<Tag, Value>, registry: CustomTagRegistry) {
        // lots of sanity checks
        for (tag, value) in tags {
            registry.parse(tag, value).await?;
            // do we remove them from the rest?
        }
        Self {
            tags,
            registry
        }
    }
}
```

## Overview skipping?

Ok, so why this crate was initially made was:

1. Reading a specific set of tiles at a given overview level as quickly as possible
2. Mapping application: Reading overlapping tiles of a bbox at given overview level
3. bevy_terrain: acting as a tile server over multiple overview levels using a quadtree

and the `CogCache` works superduper well for 3, or any other application, but 1
needs some more love. I don't like the `Ifd{BTreeMap<Tag,Enum{Offset,Value}>}`
because it complicates internal IFD structure. Also I think this skipping
behaviour is somewhat specific and I don't really want to cater for that

The problem is that we need fine-grained control, since we'd only want to read
the geo tags from the first ifd, and skip the `TileOffset/ByteCounts` (or
any other out-of-offset values really).

Here, there's a tension, because I really don't want `MetaReader` as a trait,
because having such a central thing a trait is really really bad, because I
cannot add functions without it being breaking.

So the conclusion is somewhat that we want users to easily make their own
readers _IF THEY REALLY REALLY WANT TO_, making the `MetaParser` have a nice
api. Then they can also make synchronous readers if they'd want.

Ok, so that's kinda ok

## Encoding?

So I want to play a game.

the game uses and wants to cache tiles. PNG can't store floats, and maybe I want
to use a geopackage, so yeah, let's do some stuff. Ideally, we'd write all tiffs
as cogs, because they are just beautiful.

There, the `Ifd{BTreeMap<Tag,Enum{Offset,Value}>}` actually works quite well,
because it somehow also keeps track of state for which tags are written or not.

### making a planning

The first step in writing is planning where to write what. We know:

```
|header|ifd0|geo|ifd1|TileOffsets/ByteCounts|image1|image0|
```

and I think it's perfectly fine to completely error if we're trying to write a
smalltiff larger than 4 GB.

So... can we know this? Especially with the CustomTagParser....

Ideally, we'd give an IFD a function like `byte_size` which is the total byte
size and then we can just do a:

```
let offsets = ifd.tile_offsets.len() * if self.big {8} else {4};
let byte_counts = ifd.tile_byte_counts.len() * if self.big{4} else {2}; //if I'm correct?
let if_front_size = ifd.byte_size() - offsets - byte_counts;
```

that's kinda ok, and then keep a tracker or something for where we can start/end:

```rust
struct Ifd {
    file_offset: u64
}
```

which can be updated

To be honest, I don't really know which tags would all be in each ifd, but I
think it's safe to assume they are deterministic.

#### compression

When compression is used, we really need to start at the lowest-resolution
subfile, because we don't know how much those tiles can be compressed, so this
ordering in writing is needed:

```
                         4        3           1      2
|header|ifd0|geo|ifd1|TileOffsets/ByteCounts|image1|image0|
\-----anytime-------/
```

#### The special non-compressed case

In this case, the entire tiff is deterministic and we can write pixels for each
overview together with the full-resolution tile _iff tiles are powers of 2_.

I had an image somewhere of how this works....

but then why not have a directory of tiles or something? I would separate
compression and generation steps for direct overview creation tbh.

#### Keeping track of what is written

So GDAL does this by having 0-valued entries in `TileByteCounts/Offsets`, I guess that's fine.

We can also know location of tiff tags in the file at any time, the only thing is that we don't store offset yet

```rust
struct CogEncoder {

}

```

#### `CustomTag`s

The simple case is e.g. GeoKeys, where the tags just translate back to real tags.

The complex case is when there is some IFD structure in the custom tag, like with EXIF

I don't know what OME does tbh, maybe they'll complain at some point that they want a chance to re-edit their write

```rust
/// Trait for writing your custom tags
pub trait CustomTagWriter: CustomTagParser {
    /// Encode this tag
    fn encode(&mut self, Tag) -> Value;
    /// additional writing to be done
    /// 
    /// This will always be called before encode, so that you get the chance to set offsets and such
    fn extra_write(&mut self, location: u64) -> Bytes; // or hand it a writer? no, they could totally abuse that responsibility and write in weird places
    // yeah, we could also just give them the offset at some point
}
```

#### api structure

So, we have the decoding pipeline:

```
+----------+
|MetaParser|  synchronous, stateless, parses stuff based on passed-in ranges
+----------+
      |
      |
     \/
/---------\
|MetaReader| async-sync interface determines whether to read IFDs concurrently or not
\---------/
     |
     |
     \/
 /-------\
|MetaCache| async, structure-aware caching layer
 \-------/
     |
    \/
(MetaFetch) async, just talks to the network layer
```

but for encoding it looks a bit different....

```
+----------+
|TileSource| generates tiles to be written into cog, driving force
+----------+
     |
|compressor| compresses tiles/stufff
     |
/----------\
|CogPlanner| keeps track of which tiles have been written where
\----------/ Also ensures metadata stuff is written
     |
 /--------\  do we need this? I think it actually makes sense
|WriteCache| to put it one layer higher, since it keep a 
 \--------/  cache and makes pyramids
     |
  (Push) dumb network layer
```

Now it would be really nice to make like a visualization tool for writing tiles and pyramids. The cool thing is that it is totally super efficient with a Hilbert curve, or any curve of the space-filling type. Like first I tried it with the classic z-curve:

```
0 1    -/
2 3    /-
```

which then gets bigger:

```
 numbers   full      big       small
0 1  4 5  -/ /-/    -----/    -/  -/
2 3  6 7  /-/ //        /     /-  /-
           /<-/       /
8 9  C D  // /-/     /        -/  -/
A B  E F  /-/ /-    /------   /-  /-
```

So the idea is that full is the small ones connected in big order. The cool thing about this is that the first four tiles already make the first-level pyramid, so then those don't need to be kept in-memory anymore:

```
no levels yet
0 1
2 
first 1-level
 0   1 2
     3 
two 1-level
0    1


2 3
4
three 1-level
0     1


2     3 4
      5
four 1-level = one 2-level
           1 2
           3
    0
```

etc.
This is much better than row-by-row:

```
no level yet (tiles_per_row + 1)
0 1  2 3   4 5  6 7
8
first 1-level
0    1 2   3 4  5 6
     7
second 1-level
0    1     2 3  4 5
           6
third 1-level
0    1     2    3 4
                5
fourth 1-level
0    1     2    3


4 5  6 7   8 9  A B
C
fifth 1-level
0    1     2    3


4    5 6   7 8  9 A
     B
sixth (tiles_per_row * 2 + 2) 1-level = 2-level
0          1    2


           3 4  5 6
           7
seventh
0          1    2


           3    4 5
                6
eighth
0          1





2    3     4    5


6 7  8 9   A B  C D
E
eighth
0          1





2          3    4


           5 6  7 8
           
```

So yeah, the worst-case number of tiles in memory, for `n` pyramid levels in this case is....

\[
\texttt{tiles\_per\_row}[n+1] = \left\lceil\frac{\texttt{tiles\_per\_row}[n]}{2}\right\rceil
\]

Which is annoyingly nonlinear, but in the square case, there was a square root for the error term which worked quite well... Ah yes, that's because the square root was an estimate for `tiles_per_row`, so we can safely add `1` as an upper bound...

\[
&\sum_{n=0}^N\frac{\texttt{tiles\_per\_row}[0]+1}{2^n}\\
&=\frac{\texttt{tiles\_per\_row}[0]+1}{1-\frac{1}{2}}\sum_{n=0}^N\frac{1}{2^n}\left(1-\frac{1}{2}\right)\\
&=\frac{C}{1-\frac{1}{2}}\left(1\left(1-\frac{1}{2}\right)+\frac{1}{2}\left(1-\frac{1}{2}\right)\cdots \frac{1}{2^N}\left(1-\frac{1}{2}\right)\right)\\
&= 2C\left(1-\frac{1}{2}+\frac{1}{2}-\frac{1}{4}+\cdots -\frac{1}{2^N}\right)\\
&=2C(1-\frac{1}{2^N})\\
= C &= 2(\texttt{tiles\_per\_row}[0]+1)(1-\frac{1}{2^N})
\]
where $N$ is the number of pyramids. OK, that's manageable!

Now, let's take the 30m Africa cog example and assume Africa is approximately square Africa $30\_370\_000 km²$ so $183\_697$ tiles per row. And $15$ pyramid levels (I checked the file) Let's say singleband `f32` (or 4-band `u8`) is 256 kiB per tile uncompressed, so $2(183\_698)(2-\frac{1}{2^{16}})256\text{kiB}=90\text{GiB}$ vs $12$MiB when following a hilbert curve. That's cool (I'm actually pretty excited at this point).

Now, the age-old question: What does GDAL do?

that's for another time folks.

The other time is now, GDAL [in its ghost area](https://gdal.org/en/stable/drivers/raster/cog.html#header-ghost-area) has a key that says "BLOCK_ORDER=ROW_MAJOR" with really not any other options. There's [this crate](https://github.com/paulchernoch/hilbert) for hilbert curves with some comments that made me re-evaluate my interactions on repsitories earlier. (poor maintainer).
