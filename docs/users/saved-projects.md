# Saved projects

A **project** is a name and a path, written down once and used by name
afterwards. The spawn form's Repository field takes either: a saved project's
name, or a path typed out in full.

## Where the file lives

```
$XDG_CONFIG_HOME/harness-launcher/config.toml
~/.config/harness-launcher/config.toml       # when XDG_CONFIG_HOME is unset
```

The app reads this file at start-up. It never creates it and never writes to
it. Having no file at all is the ordinary case, and the form then takes paths
only.

## What it holds

```toml
[projects]
launcher = "/home/you/code/harness-launcher"
notes = "/home/you/code/notes"
"the api" = "/home/you/work/api-server"
```

- One line per project, under the `[projects]` heading.
- The key is the name you type in the form. Quote it if it has a space in it.
- The value is a git repository, or any directory inside one. Write an
  absolute path. A relative one is resolved against the directory the app was
  started in.
- Reading the file checks no path. A path that is not a git repository is
  refused when you press `F5`, with the reason under the form.

## A file the app cannot read

The app refuses at start-up and prints the file name and what went wrong. A
hand-written file with a mistake in it is something to fix. A heading other
than `[projects]` is refused for the same reason, so a mistyped heading is not
read as a file with no projects in it.

## Using a project in the form

Type any part of the name into the Repository field. The names it matches
appear under the field while the keyboard is in it, and `↑` / `↓` move between
them.

- The typed characters have to appear in the name in that order, and case is
  ignored. Any number of characters may sit between them: `cla` and `cld` both
  match `Clade`, and `cled` does not.
- Matches are ordered by how far into the name the last typed character falls,
  nearest the front first. Names that tie are in alphabetical order, with case
  ignored there too.
- Nothing is marked until you move onto a suggestion with `↑` or `↓`. The `›`
  mark says which name `F5` will use. Typing again takes the mark away, because
  the new text matches a different set of names.

`F5` reads the field like this:

- A name typed out in full is that project's path, marked or not. Case is
  ignored here as it is in the matching, so `clade` and `Clade` are one name.
- A suggestion you moved onto is that project's path.
- Anything else is a path, and it is used exactly as you typed it. That covers
  a path typed out, a relative path, and part of a name you did not move onto —
  the form starts a spawn on the repository you named, and never picks between
  matches for you.
