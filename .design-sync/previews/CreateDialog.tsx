import { CreateDialog } from '@takomo/web'

const noop = () => {}

const LABELS = {
  title: 'New initiative',
  subtitle:
    'An idea that is not yet work. A title and a sentence are enough — everything else grows through entries.',
  fTitle: 'Title',
  fTitlePh: 'What is the idea called?',
  fSummary: 'Short description',
  fSummaryPh: 'One or two sentences.',
  fLabels: 'Labels',
  fLabelsPh: 'comma-separated',
  fTags: 'Tags',
  fTagsPh: 'person:ada, component:billing',
  create: 'Create',
  cancel: 'Cancel',
  needTitle: 'A title is required.',
}

/**
 * The whole create flow, open. Labels and tags are comma-separated text rather
 * than a picker: the vocabulary is open, and a picker would imply it is not.
 */
export function Open() {
  return (
    <CreateDialog
      open
      onOpenChange={noop}
      project="takomo"
      onCreate={noop}
      onInvalid={noop}
      labels={LABELS}
    />
  )
}
