function useExample() {
  return useMutation({
    mutationFn: async () => {
      let result;
      await Promise.all([
        async () => {
          result = await load();
        },
      ]);
      return result;
    },
  });
}
