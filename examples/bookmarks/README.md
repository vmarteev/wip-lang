# bookmarks

Writes a page of bookmarks, or an RSS feed of them, from a file of one a
line: `title | link | tags | note`.

```sh
wip run examples/bookmarks/main.wip examples/bookmarks/bookmarks.txt
wip run examples/bookmarks/main.wip -t "My links" -o links.html examples/bookmarks/bookmarks.txt
wip run examples/bookmarks/main.wip --rss https://example.com examples/bookmarks/bookmarks.txt
wip test examples/bookmarks/main.wip
```

What it shows:

- **A literal builds the type expected of it.** `page` and `item` answer
  `Html`, so their literals are `Html`, and every value in a hole — a
  title, a link, a tag, a note, all read from the file — is escaped as its
  place in the page needs, with no call to say so. `bookmarks.txt` has a
  title with `<` and `&` in it, which show on the page as written.
- **A link from outside runs nothing.** One bookmark's link is
  `javascript:alert(document.cookie)`. In an `href`, its `:` is written
  `%3A`, so it is a relative path, not a script; `https:` and `mailto:`
  links are kept.
- **A page is built from pieces.** `item` is a function that answers
  `Html`, and the page joins each one as it is, between tags; the tags are
  links joined with `Html::joined(&tags, ", ")`. A note's backticks become
  `<code>`, its text escaped around them.
- **A local says what it is.** `val link: Html = "<a …>"` is HTML. With no
  type written it would be a `String`, text, and joining it would escape
  its tags, which shows on the page rather than letting text in as markup.
- **`Xml` for the feed,** the same literals, escaped as XML is.
- **Options and errors** with `std::args`, as `wordcount` has them.
