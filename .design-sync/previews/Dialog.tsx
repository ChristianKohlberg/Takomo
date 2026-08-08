import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@takomo/web'

// Rendered OPEN. A closed dialog paints nothing, so a card of one would be a
// blank card — `cfg.overrides.Dialog` pins cardMode "single" with a viewport so
// the open state renders inside the card instead of escaping it.

/** The compound, open: header, description, footer actions. */
export function Open() {
  return (
    <Dialog open>
      <DialogContent className="max-w-140">
        <DialogHeader>
          <DialogTitle>New initiative</DialogTitle>
          <DialogDescription>
            An idea that is not yet work. A title and a sentence are enough — everything else grows
            through entries.
          </DialogDescription>
        </DialogHeader>
        <p style={{ margin: 0, fontSize: 13.5, color: 'var(--ink2)' }}>
          It will be filed under <strong>takomo</strong> and start as <strong>open</strong>.
        </p>
        <DialogFooter>
          <Button variant="outline">Cancel</Button>
          <Button>Create</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
