function Component({flag}) {
  let count = 0;
  const mutate = () => count++;
  const run0 = () => mutate;
  const run1 = () => (flag ? run0() : run0());
  const run2 = () => (flag ? run1() : run1());
  const run3 = () => (flag ? run2() : run2());
  const run4 = () => (flag ? run3() : run3());
  const run5 = () => (flag ? run4() : run4());
  const run6 = () => (flag ? run5() : run5());
  const run7 = () => (flag ? run6() : run6());
  const run8 = () => (flag ? run7() : run7());
  const run9 = () => (flag ? run8() : run8());
  const run10 = () => (flag ? run9() : run9());
  const run11 = () => (flag ? run10() : run10());
  const run12 = () => (flag ? run11() : run11());
  const run13 = () => (flag ? run12() : run12());
  const run14 = () => (flag ? run13() : run13());
  const run15 = () => (flag ? run14() : run14());
  const run16 = () => (flag ? run15() : run15());
  const run17 = () => (flag ? run16() : run16());
  const run18 = () => (flag ? run17() : run17());
  const run19 = () => (flag ? run18() : run18());
  const run20 = () => (flag ? run19() : run19());
  run20()();
  return count;
}
