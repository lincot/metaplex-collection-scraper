$(document).ready(() => {
  $.getJSON(
    "collections/BUjZjAS2vbbb65g7Z1Ca9ZRVYoJscURG5L3AkVvHP9ac.json",
    (data) => {
      const table = $("#tokenTable").DataTable({
        data: data.tokens,
        columns: [
          { data: "image__", title: "image" },
          { data: "name__", title: "name" },
        ].concat(
          data.trait_types.map((t) => ({
            data: t,
            title: t,
            defaultContent: "",
          }))
        ),
        dom: "PBlfrtip",
        select: { style: "multi" },
        buttons: [
          {
            text: "Select All Filtered",
            action: () => {
              table.rows({ filter: "applied" }).select();
            },
          },
          "selectNone",
          {
            text: "Create Merkle Tree",
            extend: "selected",
            action: () => {
              const selectedData = table.rows({ selected: true }).data();
              let mints = [];
              for (let i = 0; i < selectedData.length; i++) {
                mints.push(selectedData[i].mint_address);
              }
              $("#chosenTokens").html(
                "Here's the set of mint addresses to be hosted on arweave:<pre>" +
                  JSON.stringify(mints, null, "  ") +
                  "<pre/>"
              );
            },
          },
        ],
        searchPanes: { cascadePanes: true, initCollapsed: true },
        autoWidth: false,
        columnDefs: [
          {
            width: "0px",
            targets: [0],
            searchPanes: { show: false },
            render: (url) => "<img src='" + url + "' loading='lazy' />",
          },
          { searchPanes: { show: false }, targets: [1] },
          { targets: "_all", searchPanes: { show: true } },
        ],
        deferRender: true,
      });
    }
  );
});
