const months = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

const two = (value: number) => String(value).padStart(2, "0");

export const clockTime = (date: Date) =>
  `${two(date.getHours())}:${two(date.getMinutes())}`;

export const dayMonth = (date: Date) =>
  `${two(date.getDate())} ${months[date.getMonth()]}`;

// Desktop mail-row date: the time today, otherwise day and month.
export function rowDate(date: Date, now = new Date()) {
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return sameDay ? clockTime(date) : dayMonth(date);
}

// Desktop reader date line, shown above the time.
export const readerDate = (date: Date) =>
  `${dayMonth(date)} ${date.getFullYear()}`;
