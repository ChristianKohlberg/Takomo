// The rename modal. Three behaviours worth pinning: it says what the name IS
// every time it opens, it refuses to save nothing, and a rename that changes
// nothing is a close rather than a write.
import { describe, it, expect, afterEach, vi } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import { RenameDialog } from './RenameDialog'

afterEach(cleanup)

const labels = {
  title: 'Rename initiative',
  subtitle: 'A new name for this initiative.',
  field: 'Title',
  placeholder: 'What is the idea called?',
  save: 'Save',
  cancel: 'Cancel',
  needTitle: 'A title is required.',
}

function show(over: Partial<Parameters<typeof RenameDialog>[0]> = {}) {
  const onRename = vi.fn()
  const onInvalid = vi.fn()
  const onOpenChange = vi.fn()
  const view = render(
    <RenameDialog
      open
      onOpenChange={onOpenChange}
      value="Billing"
      subject="ini-a"
      onRename={onRename}
      onInvalid={onInvalid}
      labels={labels}
      {...over}
    />,
  )
  return { onRename, onInvalid, onOpenChange, view }
}

const field = () => screen.getByLabelText(labels.field) as HTMLInputElement

describe('RenameDialog', () => {
  it('opens on the current name, and says which one it is renaming', () => {
    show()
    expect(field().value).toBe('Billing')
    expect(screen.getByText('ini-a')).toBeTruthy()
  })

  it('saves a trimmed new name', () => {
    const { onRename, onOpenChange } = show()
    fireEvent.change(field(), { target: { value: '  Billing v2  ' } })
    fireEvent.click(screen.getByRole('button', { name: labels.save }))
    expect(onRename).toHaveBeenCalledWith('Billing v2')
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('saves on Enter, so the gesture ends where it started', () => {
    const { onRename } = show()
    fireEvent.change(field(), { target: { value: 'Renamed' } })
    fireEvent.keyDown(field(), { key: 'Enter' })
    expect(onRename).toHaveBeenCalledWith('Renamed')
  })

  it('refuses an empty title instead of clearing the name', () => {
    const { onRename, onInvalid } = show()
    fireEvent.change(field(), { target: { value: '   ' } })
    fireEvent.click(screen.getByRole('button', { name: labels.save }))
    expect(onRename).not.toHaveBeenCalled()
    expect(onInvalid).toHaveBeenCalledWith(labels.needTitle)
  })

  // A toast claiming a saved title after a no-op is a lie about what happened,
  // and the request behind it is a version bump for nothing.
  it('closes without writing when the name did not change', () => {
    const { onRename, onOpenChange } = show()
    fireEvent.click(screen.getByRole('button', { name: labels.save }))
    expect(onRename).not.toHaveBeenCalled()
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('re-seeds the field when it reopens, discarding a cancelled edit', () => {
    const { view } = show()
    fireEvent.change(field(), { target: { value: 'Half-typed' } })
    view.rerender(
      <RenameDialog
        open={false}
        onOpenChange={() => {}}
        value="Billing"
        onRename={() => {}}
        onInvalid={() => {}}
        labels={labels}
      />,
    )
    view.rerender(
      <RenameDialog
        open
        onOpenChange={() => {}}
        value="Billing"
        onRename={() => {}}
        onInvalid={() => {}}
        labels={labels}
      />,
    )
    expect(field().value).toBe('Billing')
  })
})
