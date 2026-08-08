import { CreateScheduleDialog } from '@takomo/web'

const noop = () => {}

/**
 * Recurring work, declared once. The cadence fields change with the unit —
 * weekdays for a weekly schedule, a day-of-month for a monthly one — so the form
 * never asks for a field that cannot apply.
 */
export function Open() {
  return (
    <CreateScheduleDialog
      open onOpenChange={noop} project="demo" onCreate={async () => {}}
      labels={{
        title: 'New schedule',
        subtitle: 'Work that should recur. Each due slot files exactly one ticket — never two, even if nothing picked the last one up.',
        fName: 'Name', fNamePh: 'Weekly dependency audit',
        fEvery: 'Every', fInterval: 'Interval', fDays: 'On', fDayOfMonth: 'Day of month',
        fAt: 'At', fTz: 'Time zone',
        fTitle: 'Ticket title', fTitlePh: 'Audit dependencies for advisories',
        fTitleHint: 'The title every filed ticket gets.',
        fBody: 'Ticket body',
        fLabels: 'Labels', fLabelsPh: 'ops, security',
        fRationale: 'Why', fRationalePh: 'Advisories land continuously; a weekly sweep keeps the window short.',
        unitDay: 'day', unitWeek: 'week', unitMonth: 'month',
        create: 'Create', cancel: 'Cancel', pickDay: 'Pick at least one day.',
      }}
    />
  )
}
